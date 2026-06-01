/**
 * Foni — pi TTS extension entry point.
 *
 * This file is a thin adapter: it wires pi lifecycle events and commands
 * to FoniEngine. No domain logic lives here.
 *
 * Domain:  core/engine.ts  (FoniEngine)
 * Config:  core/config.ts  (FoniConfig, DEFAULT_CONFIG)
 * Stream:  core/stream.ts  (drainChunks, StreamState)
 * TUI:     tui/foni-panel.ts, tui/model-picker.ts
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

import { FoniEngine, type FacadeFactory, type TranslatorFactory, type ProcessorFactory } from "./core/engine.ts";
import { DEFAULT_CONFIG }    from "./core/config.ts";
import type { FoniConfig }   from "./core/config.ts";
import {
  IdentityTranslator, LibreTranslateTranslator, PipelineTranslator,
  makeTranslateMiddleware, makeMatMiddleware, makeInterjectMiddleware, makeITGlossaryMiddleware,
} from "./pipeline/translators.ts";
import { BIAS_WORDS, effectiveWeights, currentIntensity } from "./core/emotion.ts";
import type { EmotionState, Emotion } from "./core/emotion.ts";
import { IdentityProcessor, RVCProcessor, SmoothingProcessor } from "./pipeline/processors.ts";
import { SynthBackend }      from "./pipeline/synth-backend.ts";
import type { SynthBackendOpts } from "./pipeline/synth-backend.ts";
import { syncBreaksFrom }    from "./pipeline/prosody.ts";
import { BreathProcessor }   from "./pipeline/breath-injector.ts";
import { SpeakFacade }       from "./pipeline/speak-facade.ts";
import { SystemPlayer }      from "./pipeline/player.ts";
import { SileroBackend }     from "./backends/silero.ts";
import { KokoroBackend }     from "./backends/kokoro.ts";
import { FakeYouBackend }    from "./backends/fakeyou.ts";
import { EspeakBackend }     from "./backends/espeak.ts";

// ─── Factory implementations (index.ts is the composition root) ───────────────────
//
// All concrete backend/processor construction lives here, not in core/engine.ts.
// This is the Composition Root pattern: one place wires everything together.

// ─── Factory implementations (index.ts is the composition root) ───────────────
//
// All concrete construction lives here. FoniEngine receives only abstractions.
// This is the full Composition Root pattern: one place wires everything together.

const translatorFactory: TranslatorFactory = (cfg, emotion) => {
  const ew = effectiveWeights(emotion);
  const matProb       = Math.min(1, cfg.matProb       * ew.matMultiplier);
  const interjectProb = Math.min(1, cfg.interjectProb * ew.interjectMultiplier);
  const bias          = ew.wordBias;
  const stack = [makeITGlossaryMiddleware()];
  if (cfg.inputLang !== cfg.outputLang) {
    stack.push(makeTranslateMiddleware(cfg.inputLang, cfg.outputLang));
  }
  if (cfg.outputLang === "ru") {
    if (cfg.matEnabled)       stack.push(makeMatMiddleware(matProb, cfg.matStretch, bias, BIAS_WORDS));
    if (cfg.interjectEnabled) stack.push(makeInterjectMiddleware(interjectProb, bias, BIAS_WORDS));
  }
  return new PipelineTranslator(stack, cfg.outputLang);
};

const RATE_NEUTRAL  = 100;

function lerpRound(a: number, b: number, t: number): number {
  return Math.round(a + t * (b - a));
}

/** Compute espeak prosody overrides from the current emotion state. */
function emotionProsody(
  emotion: Emotion,
  intensity: number,
): { ratePct: number; range: string } {
  switch (emotion) {
    case "angry":      return { ratePct: lerpRound(RATE_NEUTRAL, 90,  intensity), range: "x-high" };
    case "frustrated": return { ratePct: lerpRound(RATE_NEUTRAL, 97,  intensity), range: "high"   };
    case "sarcastic":  return { ratePct: lerpRound(RATE_NEUTRAL, 95,  intensity), range: "x-high" };
    case "excited":    return { ratePct: lerpRound(RATE_NEUTRAL, 107, intensity), range: "high"   };
    case "cute":       return { ratePct: lerpRound(RATE_NEUTRAL, 104, intensity), range: "medium"  };
    default:           return { ratePct: RATE_NEUTRAL, range: "medium" };
  }
}

/** Build SynthBackendOpts, blending emotion state into prosody overrides. */
function synthBackendOpts(cfg: typeof DEFAULT_CONFIG, emotion: EmotionState): SynthBackendOpts {
  const base: SynthBackendOpts = {
    url:     cfg.rvcUrl,
    model:   cfg.rvcModel,
    prosody: cfg.prosodyEnabled && cfg.outputLang === "ru",
  };
  const intensity = currentIntensity(emotion);
  if (intensity < 0.3 || emotion.emotion === "neutral") return base;
  return { ...base, ...emotionProsody(emotion.emotion, intensity) };
}

const facadeFactory: FacadeFactory = async (cfg, translator, emotion) => {
  // Fast path: when foni-synth is up and RVC is enabled, route all synthesis
  // through a single POST /synthesize — SSML + espeak + RVC + DSP in Rust.
  if (cfg.rvcEnabled && cfg.rvcModel) {
    const synth = new SynthBackend(synthBackendOpts(cfg, emotion));
    if (await synth.isAvailable()) {
      syncBreaksFrom(cfg.rvcUrl); // fire-and-forget: keeps prosody.ts in sync with Rust
      return new SpeakFacade(translator, synth, new IdentityProcessor(), new SystemPlayer(), {
        voice: cfg.voice, speed: cfg.speed,
      });
    }
  }

  // Fallback: legacy espeak + optional Rust DSP chain.
  const candidates = [
    new SileroBackend(cfg.sileroUrl),
    new KokoroBackend(cfg.kokoroUrl),
    new FakeYouBackend(cfg.fakeyouToken, cfg.fakeyouApiKey),
    new EspeakBackend(cfg.outputLang === "ru" ? "ru" : "en"),
  ];
  let backend = null;
  if (cfg.backendPref !== "auto") {
    const preferred = candidates.find(b => b.name === cfg.backendPref);
    if (preferred && await preferred.isAvailable()) {
      backend = preferred;
    }
  } else {
    for (const b of candidates) {
      if (await b.isAvailable()) {
        backend = b;
        break;
      }
    }
  }
  if (!backend) return null;
  const processor = processorFactory(cfg);
  return new SpeakFacade(translator, backend, processor, new SystemPlayer(), {
    voice: cfg.voice, speed: cfg.speed,
  });
};

const processorFactory: ProcessorFactory = (cfg: FoniConfig) => {
  if (!cfg.rvcEnabled || !cfg.rvcModel) return new IdentityProcessor();
  const inner = new SmoothingProcessor(new RVCProcessor(cfg.rvcUrl), {}, cfg.rvcUrl);
  return cfg.breathEnabled ? new BreathProcessor(inner, {}, cfg.rvcUrl) : inner;
};
import { pickModel }         from "./tui/model-picker.ts";
import { openFoniPanel }     from "./tui/foni-panel.ts";
import type { FoniPanelState, FoniPanelActions } from "./tui/foni-panel.ts";

// ─── Extension entry point ────────────────────────────────────────────────────

export default async function (pi: ExtensionAPI) {
  const engine = new FoniEngine(
    { ...DEFAULT_CONFIG },
    facadeFactory,
    translatorFactory,
    processorFactory,
  );
  const config = engine.config;

    // ── Mixer session ───────────────────────────────────────────────────────────

  interface MixerTrack {
    label: string; rating?: number; winner?: boolean;
    note?: string; opts: Record<string, number>; gap_pct?: number;
  }
  interface MixerSession {
    phrase: string; model: string; saved_at: string;
    tracks: MixerTrack[];
    ai_suggestions?: Array<{ label: string; rationale: string; opts: Record<string, number> }>;
  }

  const SESSION_PATH = `${process.env["HOME"] ?? "~"}/.local/share/foni/mixer-session.json`;
  let mixerSession: MixerSession | null = null;

  async function loadMixerSession(): Promise<void> {
    try {
      const { readFile } = await import("node:fs/promises");
      mixerSession = JSON.parse(await readFile(SESSION_PATH, "utf8")) as MixerSession;
    } catch { mixerSession = null; }
  }

  async function saveMixerSession(session: MixerSession): Promise<void> {
    try {
      const { mkdir, writeFile } = await import("node:fs/promises");
      const dir = SESSION_PATH.replace(/\/[^\/]+$/, "");
      await mkdir(dir, { recursive: true });
      await writeFile(SESSION_PATH, JSON.stringify(session, null, 2));
    } catch { /* best-effort */ }
  }

  function mixerWinner(): MixerTrack | undefined {
    return mixerSession?.tracks.find(t => t.winner);
  }

  function mixerContext(): string {
    if (!mixerSession) return "No mixer session loaded.";
    const winner = mixerWinner();
    const rated  = mixerSession.tracks.filter(t => t.rating !== undefined);
    return [
      `Mixer QA session — phrase: «${mixerSession.phrase}»`,
      `Saved: ${mixerSession.saved_at}`,
      "",
      "Rated tracks (human perceptual judgment):",
      ...rated.map(t =>
        `  ${"★".repeat(t.rating ?? 0)}${"☆".repeat(5 - (t.rating ?? 0))} ${t.label}${
          t.winner ? " ✪ winner" : ""}${t.note ? ` — ${t.note}` : ""}`
      ),
      ...(winner ? ["", `Winner DSP opts (${winner.label}):`,
        JSON.stringify(winner.opts, null, 2)] : []),
    ].join("\n");
  }

// ── Status bar ─────────────────────────────────────────────────────────────

  function updateStatus(ctx: { ui: { setStatus: Function; theme: any } }): void {
    const { theme } = ctx.ui;
    if (!config.enabled) { ctx.ui.setStatus("tts", undefined); return; }
    const s = engine.status();
    const lang = s.inputLang === s.outputLang
      ? s.outputLang.toUpperCase()
      : `${s.inputLang.toUpperCase()}→${s.outputLang.toUpperCase()}`;
    const mat = s.matEnabled ? "+mat" : "";
    const ij  = s.interjectEnabled ? "+oj" : "";
    const emotion = s.emotionEmoji ? ` ${s.emotionEmoji}` : "";
    const winner   = mixerWinner();
    const mix      = winner ? theme.fg("accent", ` ✪${winner.label}`) : "";
    ctx.ui.setStatus(
      "tts",
      theme.fg("accent", "TTS") + theme.fg("dim", ` ${s.backendName}${s.rvcModel ? `+${s.rvcModel}` : ""}${mat}${ij} ${lang}${emotion}`) + mix,
    );
  }

  // ── Lifecycle events ───────────────────────────────────────────────────────

  pi.on("session_start", (_event, ctx) => {
    loadMixerSession().then(() => updateStatus(ctx));
    updateStatus(ctx);
    // Auto-detect RVC server
    fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(1500) })
      .then(r => r.ok ? r.json() : null)
      .then(async (data: unknown) => {
        const typed = data as { models: string[] } | null;
        if (!typed) return;
        const models: string[] = typed.models ?? [];
        if (models.length > 0 && !config.rvcModel) config.rvcModel = models[0];
        if (!config.rvcEnabled && config.rvcModel) {
          try {
            const r = await fetch(`${config.rvcUrl}/models/${encodeURIComponent(config.rvcModel)}`, {
              method: "POST", signal: AbortSignal.timeout(5_000),
            });
            if (r.ok) {
              config.rvcEnabled = true;
              engine.invalidateFacade();
            }
          } catch { /* RVC load failed */ }
        }
        updateStatus(ctx);
        engine.prewarm().then(async () => {
          try {
            const { PREWARM_RU } = await import("./core/config.ts");
            ctx.ui.notify(`Аудио кэш прогрет (${PREWARM_RU.length} фраз)`, "info");
          } catch { /* ctx may be stale in CLI sessions — ignore */ }
        });
      })
      .catch(() => {});
  });

  pi.on("message_update", (_event, _ctx) => {
    if (_event.message.role !== "assistant") return;
    const ev = _event.assistantMessageEvent;
    if (!ev || (ev as any).type !== "text_delta") return;
    engine.onDelta((ev as any).delta ?? "");
  });

  pi.on("message_end", (_event, ctx) => {
    if (_event.message.role === "user") {
      // Detect emotion from user input, update decay state, rebuild translator
      const raw  = _event.message.content;
      const text  = Array.isArray(raw)
        ? raw.map((c: any) => c?.text ?? c).filter(Boolean).join(" ")
        : (raw ?? "") as string;
      if (text) {
        engine.onUserMessage(text);
        updateStatus(ctx);
      }
      return;
    }
    if (_event.message.role !== "assistant") return;
    engine.onMessageEnd();
  });

  pi.on("agent_start", () => engine.reset());

  // ── TUI panel helpers ──────────────────────────────────────────────────────

  function panelState(): FoniPanelState {
    return {
      enabled:     config.enabled,
      inputLang:   config.inputLang,
      outputLang:  config.outputLang,
      speed:       config.speed,
      backendName: engine.status().backendName,
      backendPref: config.backendPref,
      rvcEnabled:  config.rvcEnabled,
      rvcModel:    config.rvcModel,
      rvcUrl:      config.rvcUrl,
      rvcServerOk: null,
    };
  }

  function panelActions(ctx: any): FoniPanelActions {
    return {
      toggle() {
        config.enabled = !config.enabled;
        if (config.enabled) engine.ensureFacade();
        engine.reset();
        updateStatus(ctx);
      },
      setLang(inputLang, outputLang) {
        config.inputLang  = inputLang;
        config.outputLang = outputLang;
        engine.rebuildTranslator();
        updateStatus(ctx);
      },
      setSpeed(speed) {
        config.speed = speed;
        engine.ensureFacade().then(f => f?.setOpts({ speed }));
        updateStatus(ctx);
      },
      setBackendPref(pref) {
        config.backendPref = pref as typeof config.backendPref;
        engine.invalidateFacade();
        engine.ensureFacade().then(() => updateStatus(ctx));
      },
      toggleRvc() {
        if (!config.rvcEnabled && !config.rvcModel) return;
        config.rvcEnabled = !config.rvcEnabled;
        engine.ensureFacade().then(f =>
          f?.swapProcessor(processorFactory(config)),
        );
        updateStatus(ctx);
      },
      async pickRvcModel() {
        try {
          const r = await fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(3000) });
          const models = ((await r.json()) as { models?: string[] }).models ?? [];
          if (!models.length) return;
          const picked = await pickModel(ctx, models, config.rvcModel);
          if (!picked) return;
          config.rvcModel = picked;
          const lr = await fetch(`${config.rvcUrl}/models/${encodeURIComponent(picked)}`, {
            method: "POST", signal: AbortSignal.timeout(10_000),
          });
          if (lr.ok) {
            const f = await engine.ensureFacade();
            f?.swapProcessor(processorFactory(config));
          }
          updateStatus(ctx);
        } catch { /* server unreachable */ }
      },
      async checkRvcServer() {
        try {
          const r = await fetch(`${config.rvcUrl}/params`, { signal: AbortSignal.timeout(2000) });
          return r.ok;
        } catch { return false; }
      },
    };
  }

  // ── Commands ───────────────────────────────────────────────────────────────

  pi.registerCommand("tts", {
    description: "Toggle TTS | /tts test | /tts status | /tts voice | /tts speed | /tts lang en|ru | /tts backend | /tts token | /tts rvc on|off|model|url|models | /tts mat on|off|<prob> | /tts interject on|off|<prob> | /tts stop",
    handler: async (args, ctx) => {
      const parts = (args ?? "").trim().split(/\s+/).filter(Boolean);
      const sub   = parts[0] ?? "";
      const ok    = (s: string) => `  ok  ${s}`;
      const err   = (s: string) => `  ERR ${s}`;
      const warn  = (s: string) => `  !   ${s}`;
      const on    = (k: string, v: string) => `  ok  ${k.padEnd(12)} ${v}`;
      const off   = (k: string, v: string) => `  -   ${k.padEnd(12)} ${v}`;

      // ── test ──────────────────────────────────────────────────────────────
      if (sub === "test") {
        const lines = ["Foni diagnostic:", ""];
        lines.push(config.enabled ? ok("TTS enabled") : err("TTS disabled -- run /tts to toggle on"));
        const facade   = await engine.ensureFacade();
        const backend  = { name: facade?.backendName ?? "none" };
        const player   = facade?.backendName;
        lines.push(backend ? ok(`backend: ${backend.name}`) : err("no backend -- install espeak-ng or start Silero/Kokoro"));
        if (config.inputLang !== config.outputLang) {
          const t = await new LibreTranslateTranslator(config.inputLang, config.outputLang).translate("Hello stalker");
          lines.push(t !== "Hello stalker" ? ok(`translation: "${t}"`) : warn("translation unreachable (is LibreTranslate running on :5000?)"));
        } else {
          lines.push(ok(`language: ${config.outputLang.toUpperCase()} (passthrough)`));
        }
        if (config.rvcEnabled) {
          try {
            const r = await fetch(`${config.rvcUrl}/params`, { signal: AbortSignal.timeout(2000) });
            const p = await r.json() as { current_model?: string };
            lines.push(r.ok ? ok(`RVC: ${config.rvcUrl} -- model: ${p.current_model ?? "none"}`) : err(`RVC ${r.status}`));
          } catch { lines.push(err(`RVC unreachable at ${config.rvcUrl}`)); }
        } else {
          lines.push(warn("RVC disabled -- /tts rvc on to enable bandit voice"));
        }
        ctx.ui.notify(lines.join("\n"), "info");
        if (backend && 'synthesize' in backend) {
          const testFacade = new SpeakFacade(new IdentityTranslator(), backend as import("./pipeline/interfaces.ts").TTSBackend, new IdentityProcessor(),
            (await engine.ensureFacade() as any)?.player ?? { play: async () => {}, detected: () => "none" },
            { voice: config.voice, speed: config.speed });
          await testFacade.speak("Test. One two three.", (m) => ctx.ui.notify(`  > ${m}`, "info"));
        }
        return;
      }

      // ── status ─────────────────────────────────────────────────────────────
      if (sub === "status") {
        ctx.ui.notify([
          "Foni status:", "",
          (config.enabled ? on : off)("enabled", String(config.enabled)),
          on("backend",  engine.status().backendName),
          on("voice",    config.voice),
          on("speed",    String(config.speed)),
          on("language", config.inputLang === config.outputLang
            ? config.outputLang.toUpperCase()
            : `${config.inputLang.toUpperCase()}→${config.outputLang.toUpperCase()}`),
          "",
          config.rvcEnabled ? on("rvc", `${config.rvcModel} @ ${config.rvcUrl}`) : off("rvc", "disabled"),
          off("silero", config.sileroUrl),
          off("kokoro", config.kokoroUrl),
          config.fakeyouToken ? on("fakeyou", config.fakeyouToken) : off("fakeyou", "no token"),
          "",
          on("audio cache", engine.cacheStats()),
        ].join("\n"), "info");
        return;
      }

      // ── cache ──────────────────────────────────────────────────────────────
      if (sub === "cache") {
        if (parts[1] === "clear") {
          engine.clearCache();
          ctx.ui.notify("Аудио кэш очищен", "info");
        } else {
          ctx.ui.notify(`Audio cache: ${engine.cacheStats()}\nUsage: /tts cache clear`, "info");
        }
        return;
      }

      // ── voice / speed / lang ───────────────────────────────────────────────
      if (sub === "voice") {
        config.voice = parts[1] ?? config.voice;
        engine.ensureFacade().then(f => f?.setOpts({ voice: config.voice }));
        ctx.ui.notify(`voice -> ${config.voice}`, "info");
        return;
      }
      if (sub === "speed") {
        const n = parseFloat(parts[1] ?? "");
        if (!isNaN(n) && n > 0) {
          config.speed = Math.max(0.5, Math.min(3.0, n));
          engine.ensureFacade().then(f => f?.setOpts({ speed: config.speed }));
          ctx.ui.notify(`speed -> ${config.speed}`, "info");
        } else {
          ctx.ui.notify("Usage: /tts speed <0.5-3.0>", "warning");
        }
        return;
      }
      if (sub === "lang") {
        const a = parts[1] as "en" | "ru" | undefined;
        const b = parts[2] as "en" | "ru" | undefined;
        const valid = (x?: string): x is "en" | "ru" => x === "en" || x === "ru";
        if (!valid(a)) { ctx.ui.notify("Usage: /tts lang en|ru [en|ru]", "warning"); return; }
        config.inputLang  = a;
        config.outputLang = valid(b) ? b : a;
        engine.rebuildTranslator();
        ctx.ui.notify(`language -> ${config.inputLang.toUpperCase()}→${config.outputLang.toUpperCase()}`, "info");
        updateStatus(ctx);
        return;
      }

      // ── mat ────────────────────────────────────────────────────────────────
      if (sub === "mat") {
        if (config.outputLang !== "ru") { ctx.ui.notify("Mat only works with Russian output -- /tts lang ru first", "warning"); return; }
        const matSub = parts[1] ?? "";
        if (matSub === "on" || matSub === "off") {
          config.matEnabled = matSub === "on";
          engine.rebuildTranslator();
          ctx.ui.notify(`Mat ${config.matEnabled ? `включён (prob=${config.matProb})` : "выключен"}`, "info");
          return;
        }
        if (matSub === "stretch") {
          const sp = parseFloat(parts[2] ?? "");
          if (!isNaN(sp) && sp >= 0 && sp <= 1) {
            config.matStretch = sp;
            engine.rebuildTranslator();
            ctx.ui.notify(`Mat stretch probability -> ${sp}`, "info");
          } else {
            ctx.ui.notify(`Mat stretch: ${config.matStretch}\nUsage: /tts mat stretch 0.0-1.0`, "info");
          }
          return;
        }
        const prob = parseFloat(matSub);
        if (!isNaN(prob) && prob >= 0 && prob <= 1) {
          config.matProb = prob;
          engine.rebuildTranslator();
          ctx.ui.notify(`Mat probability -> ${prob}`, "info");
          return;
        }
        ctx.ui.notify(
          `Mat: ${config.matEnabled ? "включён" : "выключен"} (prob=${config.matProb}, stretch=${config.matStretch})\n` +
          "Usage: /tts mat on|off | /tts mat 0.0-1.0 | /tts mat stretch 0.0-1.0",
          "info",
        );
        return;
      }

      // ── interject ──────────────────────────────────────────────────────────
      if (sub === "interject") {
        if (config.outputLang !== "ru") { ctx.ui.notify("Интеръекции работают только с русским выводом -- /tts lang ru", "warning"); return; }
        const ijSub = parts[1] ?? "";
        if (ijSub === "on" || ijSub === "off") {
          config.interjectEnabled = ijSub === "on";
          engine.rebuildTranslator();
          ctx.ui.notify(`Межметия: ${config.interjectEnabled ? `включены (prob=${config.interjectProb})` : "выключены"}`, "info");
          return;
        }
        const ijProb = parseFloat(ijSub);
        if (!isNaN(ijProb) && ijProb >= 0 && ijProb <= 1) {
          config.interjectProb = ijProb;
          engine.rebuildTranslator();
          ctx.ui.notify(`Межметия probability -> ${ijProb}`, "info");
          return;
        }
        ctx.ui.notify(
          `Межметия: ${config.interjectEnabled ? "включены" : "выключены"} (prob=${config.interjectProb})\n` +
          "Usage: /tts interject on|off | /tts interject 0.0-1.0",
          "info",
        );
        return;
      }

      // ── backend ────────────────────────────────────────────────────────────
      if (sub === "backend") {
        const pref = parts[1];
        if (!["silero","kokoro","fakeyou","espeak","say","auto"].includes(pref ?? "")) {
          ctx.ui.notify("Usage: /tts backend <silero|kokoro|fakeyou|espeak|say|auto>", "warning");
          return;
        }
        config.backendPref = pref as typeof config.backendPref;
        engine.invalidateFacade();
        const f = await engine.ensureFacade();
        ctx.ui.notify(f ? `backend -> ${f.backendName}` : "no backend available for that preference", f ? "info" : "warning");
        updateStatus(ctx);
        return;
      }

      // ── token (FakeYou) ────────────────────────────────────────────────────
      if (sub === "token") {
        const token = parts[1] ?? "";
        if (!token) { ctx.ui.notify("Usage: /tts token weight_xxxx", "warning"); return; }
        config.fakeyouToken = token;
        engine.invalidateFacade();
        ctx.ui.notify("FakeYou token set. Run /tts backend fakeyou to activate.", "info");
        return;
      }

      // ── search (FakeYou) ───────────────────────────────────────────────────
      if (sub === "search") {
        const query = parts.slice(1).join(" ").toLowerCase();
        if (!query) { ctx.ui.notify("Usage: /tts search <query>", "warning"); return; }
        try {
          const r = await fetch("https://api.fakeyou.com/tts/list", { signal: AbortSignal.timeout(10_000) });
          if (!r.ok) throw new Error(`HTTP ${r.status}`);
          const { models } = await r.json() as { models: Array<{ model_token: string; title: string }> };
          const hits = models.filter(m => m.title.toLowerCase().includes(query)).slice(0, 10);
          ctx.ui.notify(
            hits.length
              ? `FakeYou "${query}":\n${hits.map(m => `${m.model_token}  ${m.title}`).join("\n")}`
              : `No TTS voices found for "${query}"`,
            hits.length ? "info" : "warning",
          );
        } catch (e: any) { ctx.ui.notify(`search failed: ${e?.message}`, "warning"); }
        return;
      }

      // ── rvc ────────────────────────────────────────────────────────────────
      if (sub === "rvc") {
        const rvcSub = parts[1] ?? "";
        if (rvcSub === "on" || rvcSub === "off") {
          if (rvcSub === "on" && !config.rvcModel) { ctx.ui.notify("Set a model first: /tts rvc model <name>", "warning"); return; }
          config.rvcEnabled = rvcSub === "on";
          const f = await engine.ensureFacade();
          f?.swapProcessor(processorFactory(config));
          ctx.ui.notify(`RVC ${config.rvcEnabled ? "enabled" : "disabled"}`, "info");
          updateStatus(ctx);
          return;
        }
        if (rvcSub === "model") {
          if (!parts[2]) {
            try {
              const r    = await fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(3000) });
              const data = await r.json() as { models?: string[] };
              const models = data.models ?? [];
              if (!models.length) { ctx.ui.notify("No models on RVC server -- download one first", "warning"); return; }
              const picked = await pickModel(ctx, models, config.rvcModel);
              if (!picked) return;
              config.rvcModel = picked;
            } catch { ctx.ui.notify(`RVC unreachable at ${config.rvcUrl}`, "warning"); return; }
          } else {
            config.rvcModel = parts[2];
          }
          try {
            const r = await fetch(`${config.rvcUrl}/models/${encodeURIComponent(config.rvcModel)}`, {
              method: "POST", signal: AbortSignal.timeout(10_000),
            });
            ctx.ui.notify(r.ok ? `RVC model loaded: ${config.rvcModel}` : `RVC server ${r.status}`, r.ok ? "info" : "warning");
            if (r.ok) {
              const f = await engine.ensureFacade();
              f?.swapProcessor(processorFactory(config));
            }
          } catch { ctx.ui.notify(`RVC unreachable at ${config.rvcUrl}`, "warning"); }
          updateStatus(ctx);
          return;
        }
        if (rvcSub === "url")    { config.rvcUrl = parts[2] ?? config.rvcUrl; ctx.ui.notify(`RVC URL -> ${config.rvcUrl}`, "info"); return; }
        if (rvcSub === "models") {
          try {
            const r    = await fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(5_000) });
            const data = await r.json() as { models?: string[] } | string[];
            const list = Array.isArray(data) ? data : (data.models ?? []);
            ctx.ui.notify(list.length ? `RVC models:\n${list.join("\n")}` : "No models found", "info");
          } catch (e: any) { ctx.ui.notify(`RVC unreachable: ${e?.message}`, "warning"); }
          return;
        }
        ctx.ui.notify("Usage: /tts rvc on|off | model <name> | url <url> | models", "warning");
        return;
      }

      // ── stop ───────────────────────────────────────────────────────────────
      if (sub === "stop") { engine.stop(); ctx.ui.notify("TTS stopped", "info"); return; }

      // ── mix ─────────────────────────────────────────────────────────────────────
      if (sub === "mix") {
        const mixSub = parts[1] ?? "";

        if (mixSub === "status") {
          await loadMixerSession();
          ctx.ui.notify(mixerContext(), "info");
          updateStatus(ctx);
          return;
        }

        if (mixSub === "apply") {
          const winner = mixerWinner();
          if (!winner) { ctx.ui.notify("No winner set in mixer session. Rate tracks with fonictl mix.", "warning"); return; }
          ctx.ui.notify(
            `Applying winner DSP opts from «${winner.label}»\n` +
            JSON.stringify(winner.opts, null, 2),
            "info",
          );
          return;
        }

        if (mixSub === "suggest") {
          if (!mixerSession) { await loadMixerSession(); }
          if (!mixerSession) { ctx.ui.notify("No mixer session. Run fonictl mix first.", "warning"); return; }
          const rated   = mixerSession.tracks.filter(t => t.rating !== undefined);
          const winner  = mixerWinner();
          if (!rated.length) { ctx.ui.notify("No rated tracks yet. Use rate N 1-5 in fonictl mix.", "warning"); return; }

          // Build suggestion prompt from session context.
          const prompt  = mixerContext() + "\n\nBased on the ratings and notes above, suggest 2-3 new DSP variants to try next. " +
            "Format each as JSON: { label, rationale, opts }. opts keys match fonictl mix scratchpad params.";
          ctx.ui.notify(`⚠ Sending mixer context to agent...\n\n${prompt}`, "info");
          return;
        }

        ctx.ui.notify(
          "Usage: /tts mix status | apply | suggest\n" +
          "  status  — show current mixer QA session\n" +
          "  apply   — surface winner DSP opts\n" +
          "  suggest — ask agent to propose next experiments",
          "info",
        );
        return;
      }

      // ── toggle (no sub) / open panel ───────────────────────────────────────
      if (sub === "") {
        config.enabled = !config.enabled;
        if (config.enabled) engine.ensureFacade();
        engine.reset();
        updateStatus(ctx);
        return;
      }

      await engine.ensureFacade();
      await openFoniPanel(ctx, panelState(), panelActions(ctx));
      updateStatus(ctx);
    },
  });
}
