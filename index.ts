/**
 * Foni — Pi TTS extension.
 *
 * This file is the extension entry point only. All business logic lives in:
 *   pipeline/speak-facade.ts   — Facade
 *   pipeline/interfaces.ts     — Strategy interfaces
 *   pipeline/translators.ts    — Translator strategies
 *   pipeline/processors.ts     — AudioProcessor strategies
 *   pipeline/player.ts         — Player
 *   backends/*.ts              — TTSBackend strategies
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFileSync } from "node:child_process";
import { platform } from "node:os";

import { freshState, resolveBacktickRun, drainChunks } from "./lib.ts";
import { SpeakFacade } from "./pipeline/speak-facade.ts";
import { IdentityTranslator, MyMemoryTranslator } from "./pipeline/translators.ts";
import { IdentityProcessor, RVCProcessor } from "./pipeline/processors.ts";
import { SystemPlayer } from "./pipeline/player.ts";
import { SileroBackend } from "./backends/silero.ts";
import { KokoroBackend } from "./backends/kokoro.ts";
import { EspeakBackend } from "./backends/espeak.ts";
import { FakeYouBackend } from "./backends/fakeyou.ts";
import type { TTSBackend } from "./pipeline/interfaces.ts";
import { pickModel } from "./tui/model-picker.ts";

// ─── Config ───────────────────────────────────────────────────────────────────

const config = {
  enabled: false,
  backendPref: "auto" as "auto" | "silero" | "kokoro" | "fakeyou" | "espeak" | "say",
  voice: "en_0",
  speed: 1.0,
  lang: "en" as "en" | "ru",
  sileroUrl: "http://localhost:8001",
  kokoroUrl: "http://localhost:8880",
  fakeyouToken: "",
  fakeyouApiKey: "",
  rvcEnabled: false,
  rvcUrl: "http://localhost:5050",
  rvcModel: "",
};

// ─── Backend registry (Strategy pool) ────────────────────────────────────────

function buildBackends(): TTSBackend[] {
  return [
    new SileroBackend(config.sileroUrl),
    new KokoroBackend(config.kokoroUrl),
    new FakeYouBackend(config.fakeyouToken, config.fakeyouApiKey),
    new EspeakBackend(config.lang === "ru" ? "ru" : "en"),
  ];
}

async function detectBackend(): Promise<TTSBackend | null> {
  const backends = buildBackends();

  if (config.backendPref !== "auto") {
    const preferred = backends.find(b => b.name === config.backendPref);
    if (preferred && await preferred.isAvailable()) return preferred;
    return null;
  }

  for (const b of backends) {
    if (await b.isAvailable()) return b;
  }

  if (platform() === "darwin") {
    try { execFileSync("which", ["say"], { stdio: "ignore" }); return new EspeakBackend("en"); }
    catch { /* no say */ }
  }

  return null;
}

// ─── Facade assembly ──────────────────────────────────────────────────────────

const player = new SystemPlayer();

async function buildFacade(): Promise<SpeakFacade | null> {
  const backend = await detectBackend();
  if (!backend) return null;

  const translator = config.lang === "ru"
    ? new MyMemoryTranslator("en", "ru")
    : new IdentityTranslator();

  const processor = config.rvcEnabled && config.rvcModel
    ? new RVCProcessor(config.rvcUrl)
    : new IdentityProcessor();

  return new SpeakFacade(translator, backend, processor, player, {
    voice: config.voice,
    speed: config.speed,
  });
}

// ─── Extension ────────────────────────────────────────────────────────────────

export default async function (pi: ExtensionAPI) {
  let facade: SpeakFacade | null = null;
  let state = freshState();
  let audioQueue: Promise<void> = Promise.resolve();

  async function ensureFacade(): Promise<SpeakFacade | null> {
    if (!facade) facade = await buildFacade();
    return facade;
  }

  function enqueue(text: string): void {
    audioQueue = audioQueue.then(async () => {
      const f = await ensureFacade();
      if (f) await f.speak(text);
    });
  }

  function stopAudio(): void { audioQueue = Promise.resolve(); }

  function updateStatus(ctx: { ui: { setStatus: Function; setWidget: Function; theme: any } }): void {
    const theme = ctx.ui.theme;
    if (!config.enabled) {
      ctx.ui.setStatus("tts", undefined);
      ctx.ui.setWidget("tts", undefined);
      return;
    }
    const backend = facade?.backendName ?? "…";
    const rvc     = config.rvcEnabled && config.rvcModel ? config.rvcModel : null;
    const lang    = config.lang === "ru" ? "🇷🇺" : "🇬🇧";

    ctx.ui.setStatus(
      "tts",
      theme.fg("accent", "🔊") + theme.fg("dim", ` ${backend}${rvc ? `+${rvc}` : ""} ${lang}`)
    );

    const on  = (s: string) => `  ${theme.fg("accent", "●")} ${s}`;
    const off = (s: string) => `  ${theme.fg("dim", "○")} ${s}`;
    ctx.ui.setWidget("tts", [
      theme.fg("accent", " 🔊 Foni TTS"),
      on(`backend:  ${theme.fg("accent", backend)}`),
      rvc ? on(`rvc:      ${theme.fg("accent", rvc)}`) : off(theme.fg("dim", "rvc:      off")),
      on(`language: ${lang}`),
      on(`speed:    ${config.speed}×`),
    ]);
  }

  // ── Auto-detect RVC on startup ─────────────────────────────────────────────
  pi.on("session_start", (_event, ctx) => {
    updateStatus(ctx);
    fetch("http://127.0.0.1:5050/models", { signal: AbortSignal.timeout(1500) })
      .then(r => r.ok ? r.json() : null)
      .then((data: { models: string[] } | null) => {
        if (!data) return;
        config.rvcUrl = "http://127.0.0.1:5050";
        const models: string[] = data.models ?? [];
        if (models.length > 0 && !config.rvcModel) config.rvcModel = models[0];
        updateStatus(ctx);
      })
      .catch(() => {});
  });

  // ── Stream buffering ───────────────────────────────────────────────────────
  pi.on("message_update", (_event, _ctx) => {
    if (!config.enabled) return;
    if (_event.message.role !== "assistant") return;
    const ev = _event.assistantMessageEvent;
    if (!ev || (ev as any).type !== "text_delta") return;
    const delta: string = (ev as any).delta ?? "";
    if (!delta) return;

    for (const ch of delta) {
      if (ch === "`") { state.backtickRun++; }
      else {
        if (state.backtickRun > 0) resolveBacktickRun(state);
        if (state.codeDepth === 0 && !state.inInlineCode) state.buffer += ch;
      }
    }
    const { chunks, remainder } = drainChunks(state.buffer);
    state.buffer = remainder;
    for (const chunk of chunks) enqueue(chunk);
  });

  pi.on("message_end", (_event, _ctx) => {
    if (!config.enabled) return;
    if (_event.message.role !== "assistant") return;
    const leftover = state.buffer.trim();
    state = freshState();
    if (leftover.length > 2) enqueue(leftover);
  });

  pi.on("agent_start", () => { state = freshState(); stopAudio(); });

  // ─── Commands ──────────────────────────────────────────────────────────────

  pi.registerCommand("tts", {
    description: "Toggle TTS | /tts test | /tts status | /tts voice | /tts speed | /tts lang en|ru | /tts backend | /tts token | /tts rvc on|off|model|url|models | /tts stop",
    handler: async (args, ctx) => {
      const parts = (args ?? "").trim().split(/\s+/).filter(Boolean);
      const sub = parts[0] ?? "";
      const ok   = (s: string) => `  ✓ ${s}`;
      const err  = (s: string) => `  ✗ ${s}`;
      const warn = (s: string) => `  ⚠ ${s}`;
      const off  = (k: string, v: string) => `  ○ ${k.padEnd(12)} ${v}`;
      const on   = (k: string, v: string) => `  ✓ ${k.padEnd(12)} ${v}`;

      // ── test ────────────────────────────────────────────────────────────
      if (sub === "test") {
        const lines: string[] = ["Foni diagnostic:", ""];
        lines.push(config.enabled ? ok("TTS enabled") : err("TTS disabled — run /tts to toggle on"));
        const backend = await detectBackend();
        lines.push(backend ? ok(`backend: ${backend.name}`) : err("no backend — install espeak-ng or start Silero/Kokoro"));
        lines.push(player.detected() ? ok(`player: ${player.detected()}`) : err("no audio player (mpv / aplay / paplay)"));
        if (config.lang === "ru") {
          const t = await new MyMemoryTranslator("en", "ru").translate("Hello stalker");
          lines.push(t !== "Hello stalker" ? ok(`translation: "${t}"`) : warn("MyMemory unreachable"));
        } else {
          lines.push(ok("language: en (no translation)"));
        }
        if (config.rvcEnabled) {
          try {
            const r = await fetch(`${config.rvcUrl}/params`, { signal: AbortSignal.timeout(2000) });
            const p = await r.json() as { current_model?: string };
            lines.push(r.ok ? ok(`RVC: ${config.rvcUrl} — model: ${p.current_model ?? "none"}`) : err(`RVC ${r.status}`));
          } catch { lines.push(err(`RVC unreachable at ${config.rvcUrl}`)); }
        } else {
          lines.push(warn("RVC disabled — /tts rvc on to enable bandit voice"));
        }
        ctx.ui.notify(lines.join("\n"), "info");
        if (backend && player.detected()) {
          facade = new SpeakFacade(
            new IdentityTranslator(),
            backend,
            new IdentityProcessor(),
            player,
            { voice: config.voice, speed: config.speed },
          );
          await facade.speak("Test. One two three.", (m) => ctx.ui.notify(`  › ${m}`, "info"));
        }
        return;
      }

      // ── status ──────────────────────────────────────────────────────────
      if (sub === "status") {
        const b = facade?.backendName ?? "not detected";
        ctx.ui.notify([
          "Foni status:", "",
          (config.enabled ? on : off)("enabled",  String(config.enabled)),
          on("backend",   b),
          on("voice",     config.voice),
          on("speed",     String(config.speed)),
          on("language",  config.lang === "ru" ? "🇷🇺 ru" : "🇬🇧 en"),
          on("player",    player.detected() ?? "none"),
          "",
          config.rvcEnabled ? on("rvc", `${config.rvcModel} @ ${config.rvcUrl}`) : off("rvc", "disabled"),
          off("silero",   config.sileroUrl),
          off("kokoro",   config.kokoroUrl),
          config.fakeyouToken ? on("fakeyou", config.fakeyouToken) : off("fakeyou", "no token"),
        ].join("\n"), "info");
        return;
      }

      // ── voice / speed / lang ─────────────────────────────────────────────
      if (sub === "voice") {
        config.voice = parts[1] ?? config.voice;
        facade?.setOpts({ voice: config.voice });
        ctx.ui.notify(`voice → ${config.voice}`, "info");
        return;
      }
      if (sub === "speed") {
        const n = parseFloat(parts[1] ?? "");
        if (!isNaN(n) && n > 0) {
          config.speed = Math.max(0.5, Math.min(3.0, n));
          facade?.setOpts({ speed: config.speed });
          ctx.ui.notify(`speed → ${config.speed}`, "info");
        } else {
          ctx.ui.notify("Usage: /tts speed <0.5–3.0>", "warning");
        }
        return;
      }
      if (sub === "lang") {
        const lang = parts[1] as "en" | "ru" | undefined;
        if (lang !== "en" && lang !== "ru") { ctx.ui.notify("Usage: /tts lang en|ru", "warning"); return; }
        config.lang = lang;
        facade?.swapTranslator(lang === "ru" ? new MyMemoryTranslator("en", "ru") : new IdentityTranslator());
        ctx.ui.notify(`language → ${lang === "ru" ? "🇷🇺 Russian" : "🇬🇧 English"}`, "info");
        updateStatus(ctx);
        return;
      }

      // ── backend ──────────────────────────────────────────────────────────
      if (sub === "backend") {
        const pref = parts[1];
        if (!["silero","kokoro","fakeyou","espeak","say","auto"].includes(pref ?? "")) {
          ctx.ui.notify("Usage: /tts backend <silero|kokoro|fakeyou|espeak|say|auto>", "warning");
          return;
        }
        config.backendPref = pref as typeof config.backendPref;
        facade = null;
        const b = await detectBackend();
        if (b) { facade = await buildFacade(); ctx.ui.notify(`backend → ${b.name}`, "info"); }
        else ctx.ui.notify("no backend available for that preference", "warning");
        updateStatus(ctx);
        return;
      }

      // ── token (FakeYou) ──────────────────────────────────────────────────
      if (sub === "token") {
        const token = parts[1] ?? "";
        if (!token) { ctx.ui.notify("Usage: /tts token weight_xxxx", "warning"); return; }
        config.fakeyouToken = token;
        facade = null;
        ctx.ui.notify(`FakeYou token set. Run /tts backend fakeyou to activate.`, "info");
        return;
      }

      // ── search (FakeYou) ─────────────────────────────────────────────────
      if (sub === "search") {
        const query = parts.slice(1).join(" ").toLowerCase();
        if (!query) { ctx.ui.notify("Usage: /tts search <query>", "warning"); return; }
        try {
          const r = await fetch("https://api.fakeyou.com/tts/list", { signal: AbortSignal.timeout(10_000) });
          if (!r.ok) throw new Error(`HTTP ${r.status}`);
          const { models } = await r.json() as { models: Array<{ model_token: string; title: string; ietf_language_tag: string }> };
          const hits = models.filter(m => m.title.toLowerCase().includes(query)).slice(0, 10);
          ctx.ui.notify(hits.length
            ? `FakeYou "${query}":\n${hits.map(m => `${m.model_token}  ${m.title}`).join("\n")}`
            : `No TTS voices found for "${query}"`,
            hits.length ? "info" : "warning");
        } catch (e: any) { ctx.ui.notify(`search failed: ${e?.message}`, "warning"); }
        return;
      }

      // ── rvc ──────────────────────────────────────────────────────────────
      if (sub === "rvc") {
        const rvcSub = parts[1] ?? "";
        if (rvcSub === "on" || rvcSub === "off") {
          if (rvcSub === "on" && !config.rvcModel) { ctx.ui.notify("Set a model first: /tts rvc model <name>", "warning"); return; }
          config.rvcEnabled = rvcSub === "on";
          facade?.swapProcessor(config.rvcEnabled ? new RVCProcessor(config.rvcUrl) : new IdentityProcessor());
          ctx.ui.notify(`RVC ${config.rvcEnabled ? "enabled" : "disabled"}`, "info");
          updateStatus(ctx);
          return;
        }
        if (rvcSub === "model") {
          if (!parts[2]) {
            // No arg → interactive picker
            try {
              const r = await fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(3000) });
              const data = await r.json() as { models?: string[] };
              const models: string[] = data.models ?? [];
              if (models.length === 0) { ctx.ui.notify("No models on RVC server — download one first", "warning"); return; }
              const picked = await pickModel(ctx, models, config.rvcModel);
              if (!picked) return;
              config.rvcModel = picked;
            } catch { ctx.ui.notify(`RVC unreachable at ${config.rvcUrl}`, "warning"); return; }
          } else {
            config.rvcModel = parts[2];
          }
          if (!config.rvcModel) { ctx.ui.notify("Usage: /tts rvc model <name>", "warning"); return; }
          try {
            const r = await fetch(`${config.rvcUrl}/models/${encodeURIComponent(config.rvcModel)}`, { method: "POST", signal: AbortSignal.timeout(10_000) });
            ctx.ui.notify(r.ok ? `RVC model loaded: ${config.rvcModel}` : `RVC server ${r.status}`, r.ok ? "info" : "warning");
            if (r.ok) facade?.swapProcessor(new RVCProcessor(config.rvcUrl));
          } catch { ctx.ui.notify(`RVC unreachable at ${config.rvcUrl}`, "warning"); }
          updateStatus(ctx);
          return;
        }
        if (rvcSub === "url") { config.rvcUrl = parts[2] ?? config.rvcUrl; ctx.ui.notify(`RVC URL → ${config.rvcUrl}`, "info"); return; }
        if (rvcSub === "models") {
          try {
            const r = await fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(5_000) });
            const data = await r.json() as { models?: string[] } | string[];
            const list: string[] = Array.isArray(data) ? data : (data.models ?? []);
            ctx.ui.notify(list.length ? `RVC models:\n${list.join("\n")}` : "No models found", "info");
          } catch (e: any) { ctx.ui.notify(`RVC unreachable: ${e?.message}`, "warning"); }
          return;
        }
        ctx.ui.notify("Usage: /tts rvc on|off | model <name> | url <url> | models", "warning");
        return;
      }

      // ── stop ─────────────────────────────────────────────────────────────
      if (sub === "stop") { stopAudio(); ctx.ui.notify("TTS stopped", "info"); return; }

      // ── toggle ───────────────────────────────────────────────────────────
      config.enabled = !config.enabled;
      if (config.enabled && !facade) facade = await buildFacade();
      stopAudio();
      state = freshState();
      const icon = config.enabled ? "🔊" : "🔇";
      const label = config.enabled
        ? `TTS ON (${facade?.backendName ?? "no backend — install espeak-ng"})`
        : "TTS OFF";
      ctx.ui.notify(`${icon} ${label}`, "info");
      updateStatus(ctx);
    },
  });
}
