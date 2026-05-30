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

import { FoniEngine, type BackendFactory, type ProcessorFactory } from "./core/engine.ts";
import { DEFAULT_CONFIG }    from "./core/config.ts";
import type { FoniConfig }   from "./core/config.ts";
import { IdentityTranslator, MyMemoryTranslator } from "./pipeline/translators.ts";
import { IdentityProcessor, RVCProcessor, SmoothingProcessor } from "./pipeline/processors.ts";
import { ProsodyBackend }    from "./pipeline/prosody.ts";
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

const backendFactory: BackendFactory = async (cfg: FoniConfig) => {
  const candidates = [
    new SileroBackend(cfg.sileroUrl),
    new KokoroBackend(cfg.kokoroUrl),
    new FakeYouBackend(cfg.fakeyouToken, cfg.fakeyouApiKey),
    new EspeakBackend(cfg.outputLang === "ru" ? "ru" : "en"),
  ];
  if (cfg.backendPref !== "auto") {
    const preferred = candidates.find(b => b.name === cfg.backendPref);
    if (preferred && await preferred.isAvailable()) {
      return cfg.prosodyEnabled && cfg.outputLang === "ru"
        ? new ProsodyBackend(preferred)
        : preferred;
    }
    return null;
  }
  for (const b of candidates) {
    if (await b.isAvailable()) {
      return cfg.prosodyEnabled && cfg.outputLang === "ru"
        ? new ProsodyBackend(b)
        : b;
    }
  }
  return null;
};

const processorFactory: ProcessorFactory = (cfg: FoniConfig) => {
  if (!cfg.rvcEnabled || !cfg.rvcModel) return new IdentityProcessor();
  const inner = new SmoothingProcessor(new RVCProcessor(cfg.rvcUrl));
  return cfg.breathEnabled ? new BreathProcessor(inner) : inner;
};
import { pickModel }         from "./tui/model-picker.ts";
import { openFoniPanel }     from "./tui/foni-panel.ts";
import type { FoniPanelState, FoniPanelActions } from "./tui/foni-panel.ts";

// ─── Extension entry point ────────────────────────────────────────────────────

export default async function (pi: ExtensionAPI) {
  const engine = new FoniEngine(
    { ...DEFAULT_CONFIG },
    backendFactory,
    processorFactory,
    new SystemPlayer(),
  );
  const config = engine.config;

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
    ctx.ui.setStatus(
      "tts",
      theme.fg("accent", "TTS") + theme.fg("dim", ` ${s.backendName}${s.rvcModel ? `+${s.rvcModel}` : ""}${mat}${ij} ${lang}${emotion}`),
    );
  }

  // ── Lifecycle events ───────────────────────────────────────────────────────

  pi.on("session_start", (_event, ctx) => {
    updateStatus(ctx);
    // Auto-detect RVC server
    fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(1500) })
      .then(r => r.ok ? r.json() : null)
      .then(async (data: { models: string[] } | null) => {
        if (!data) return;
        const models: string[] = data.models ?? [];
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
        engine.prewarm().then(() =>
          ctx.ui.notify(`Аудио кэш прогрет (${(await import("./core/config.ts")).PREWARM_RU.length} фраз)`, "info"),
        );
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
      const text = (_event.message.content ?? "") as string;
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
        const backend = await engine.detectBackend();
        const player  = (await engine.ensureFacade())?.backendName;
        lines.push(backend ? ok(`backend: ${backend.name}`) : err("no backend -- install espeak-ng or start Silero/Kokoro"));
        if (config.inputLang !== config.outputLang) {
          const t = await new MyMemoryTranslator(config.inputLang, config.outputLang).translate("Hello stalker");
          lines.push(t !== "Hello stalker" ? ok(`translation: "${t}"`) : warn("translation unreachable"));
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
        if (backend) {
          const testFacade = new SpeakFacade(new IdentityTranslator(), backend, new IdentityProcessor(),
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
