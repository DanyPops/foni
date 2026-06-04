/**
 * Foni — pi TTS extension.
 *
 * Thin WebSocket adapter: forwards pi events to foni-synth Rust engine.
 * All synthesis, translation, emotion, and playback happen in Rust.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { WebSocket } from "ws";
import { pickModel } from "./tui/model-picker.ts";
import { openFoniPanel } from "./tui/foni-panel.ts";
import type { FoniPanelState, FoniPanelActions } from "./tui/foni-panel.ts";

interface FoniConfig {
  enabled: boolean;
  voice: string;
  speed: number;
  inputLang: "en" | "ru";
  outputLang: "en" | "ru";
  backendPref: "auto" | "espeak" | "say";
  matEnabled: boolean;
  matProb: number;
  matStretch: number;
  matCooldownMs: number;
  interjectEnabled: boolean;
  interjectProb: number;
  interjectCooldownMs: number;
  rvcEnabled: boolean;
  rvcUrl: string;
  rvcModel: string;
  prosodyEnabled: boolean;
}

const DEFAULT_CONFIG: FoniConfig = {
  enabled: true,
  voice: "ru",
  speed: 1.0,
  inputLang: "en",
  outputLang: "ru",
  backendPref: "espeak",
  matEnabled: true,
  matProb: 0.35,
  matStretch: 0.5,
  matCooldownMs: 20_000,
  interjectEnabled: true,
  interjectProb: 0.25,
  interjectCooldownMs: 12_000,
  rvcEnabled: true,
  rvcUrl: "http://localhost:5050",
  rvcModel: "sidorovich",
  prosodyEnabled: true,
};

// ─── Extension entry point ────────────────────────────────────────────────────

export default async function (pi: ExtensionAPI) {
  const config: FoniConfig = { ...DEFAULT_CONFIG };
  let ws: WebSocket | null = null;
  let muted = false;
  let emotionEmoji = "";

  // ── WebSocket ─────────────────────────────────────────────────────────────

  function connectWs(): void {
    const url = config.rvcUrl.replace(/^http/, "ws") + "/ws";
    try {
      ws = new WebSocket(url);
      ws.on("message", (data: Buffer) => {
        try {
          const msg = JSON.parse(data.toString());
          if (msg.type === "emotion") {
            emotionEmoji = msg.intensity >= 0.3 ? (msg.emoji ?? "") : "";
          }
        } catch { /* malformed */ }
      });
      ws.on("error", () => { ws = null; });
      ws.on("close", () => { ws = null; });
    } catch { ws = null; }
  }

  function wsSend(msg: Record<string, unknown>): void {
    if (ws?.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
    }
  }

  // ── Status bar ────────────────────────────────────────────────────────────

  function updateStatus(ctx: { ui: { setStatus: Function; theme: any } }): void {
    const { theme } = ctx.ui;
    if (!config.enabled) { ctx.ui.setStatus("tts", undefined); return; }
    const lang = config.inputLang === config.outputLang
      ? config.outputLang.toUpperCase()
      : `${config.inputLang.toUpperCase()}→${config.outputLang.toUpperCase()}`;
    const mat = config.matEnabled ? "+mat" : "";
    const ij = config.interjectEnabled ? "+oj" : "";
    const emotion = emotionEmoji ? ` ${emotionEmoji}` : "";
    ctx.ui.setStatus(
      "tts",
      theme.fg("accent", "TTS") + theme.fg("dim", ` ${muted ? "🔇 " : ""}synth${config.rvcModel ? `+${config.rvcModel}` : ""}${mat}${ij} ${lang}${emotion}`),
    );
  }

  // ── Mixer session ─────────────────────────────────────────────────────────

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

  function mixerWinner(): MixerTrack | undefined {
    return mixerSession?.tracks.find(t => t.winner);
  }

  function mixerContext(): string {
    if (!mixerSession) return "No mixer session loaded.";
    const rated = mixerSession.tracks.filter(t => t.rating !== undefined);
    return [
      `Mixer QA session — phrase: «${mixerSession.phrase}»`,
      `Saved: ${mixerSession.saved_at}`,
      "",
      "Rated tracks:",
      ...rated.map(t =>
        `  ${"★".repeat(t.rating ?? 0)}${"☆".repeat(5 - (t.rating ?? 0))} ${t.label}${
          t.winner ? " ✪ winner" : ""}${t.note ? ` — ${t.note}` : ""}`
      ),
    ].join("\n");
  }

  // ── Lifecycle events ──────────────────────────────────────────────────────

  pi.on("session_start", (_event, ctx) => {
    connectWs();
    loadMixerSession().then(() => updateStatus(ctx));
    updateStatus(ctx);
    wsSend({ type: "prewarm" });
  });

  pi.on("message_update", (_event, _ctx) => {
    if (_event.message.role !== "assistant") return;
    const ev = _event.assistantMessageEvent;
    if (!ev || (ev as any).type !== "text_delta") return;
    if (!config.enabled || muted) return;
    wsSend({ type: "delta", text: (ev as any).delta ?? "" });
  });

  pi.on("message_end", (_event, ctx) => {
    if (_event.message.role === "user") {
      const raw = _event.message.content;
      const text = Array.isArray(raw)
        ? raw.map((c: any) => c?.text ?? c).filter(Boolean).join(" ")
        : (raw ?? "") as string;
      if (text) {
        wsSend({ type: "user_message", text });
        updateStatus(ctx);
      }
      return;
    }
    if (_event.message.role !== "assistant") return;
    wsSend({ type: "message_end" });
  });

  pi.on("agent_start", () => {
    wsSend({ type: "reset" });
  });

  // ── TUI panel ─────────────────────────────────────────────────────────────

  function panelState(): FoniPanelState {
    return {
      enabled: config.enabled,
      inputLang: config.inputLang,
      outputLang: config.outputLang,
      speed: config.speed,
      backendName: "synth",
      backendPref: config.backendPref,
      rvcEnabled: config.rvcEnabled,
      rvcModel: config.rvcModel,
      rvcUrl: config.rvcUrl,
      rvcServerOk: null,
    };
  }

  function panelActions(ctx: any): FoniPanelActions {
    return {
      toggle() { config.enabled = !config.enabled; updateStatus(ctx); },
      setLang(i, o) { config.inputLang = i; config.outputLang = o; wsSend({ type: "config", key: "lang", value: `${i},${o}` }); updateStatus(ctx); },
      setSpeed(s) { config.speed = s; wsSend({ type: "config", key: "speed", value: String(s) }); updateStatus(ctx); },
      setBackendPref(p) { config.backendPref = p as any; updateStatus(ctx); },
      toggleRvc() { config.rvcEnabled = !config.rvcEnabled; updateStatus(ctx); },
      async pickRvcModel() {
        try {
          const r = await fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(3000) });
          const models = ((await r.json()) as { models?: string[] }).models ?? [];
          if (!models.length) return;
          const picked = await pickModel(ctx, models, config.rvcModel);
          if (!picked) return;
          config.rvcModel = picked;
          await fetch(`${config.rvcUrl}/models/${encodeURIComponent(picked)}`, { method: "POST", signal: AbortSignal.timeout(10_000) });
          updateStatus(ctx);
        } catch {}
      },
      async checkRvcServer() {
        try { const r = await fetch(`${config.rvcUrl}/params`, { signal: AbortSignal.timeout(2000) }); return r.ok; }
        catch { return false; }
      },
    };
  }

  // ── Commands ──────────────────────────────────────────────────────────────

  pi.registerCommand("tts", {
    description: "/tts | status | test | voice | speed | lang | mat | interject | rvc | stop | mute | mix",
    handler: async (args, ctx) => {
      const parts = (args ?? "").trim().split(/\s+/).filter(Boolean);
      const sub = parts[0] ?? "";

      if (sub === "status") {
        ctx.ui.notify([
          "Foni status:", "",
          `  enabled:   ${config.enabled}`,
          `  backend:   synth`,
          `  voice:     ${config.voice}`,
          `  speed:     ${config.speed}`,
          `  language:  ${config.inputLang === config.outputLang ? config.outputLang.toUpperCase() : `${config.inputLang.toUpperCase()}→${config.outputLang.toUpperCase()}`}`,
          config.rvcEnabled ? `  rvc:       ${config.rvcModel} @ ${config.rvcUrl}` : `  rvc:       disabled`,
          `  ws:        ${ws?.readyState === WebSocket.OPEN ? "connected" : "disconnected"}`,
        ].join("\n"), "info");
        return;
      }

      if (sub === "test") {
        try {
          const r = await fetch(`${config.rvcUrl}/params`, { signal: AbortSignal.timeout(2000) });
          ctx.ui.notify(r.ok ? "foni-synth reachable ✅" : `foni-synth HTTP ${r.status}`, r.ok ? "info" : "warning");
        } catch { ctx.ui.notify(`foni-synth unreachable at ${config.rvcUrl}`, "warning"); }
        return;
      }

      if (sub === "voice") { config.voice = parts[1] ?? config.voice; ctx.ui.notify(`voice → ${config.voice}`, "info"); return; }
      if (sub === "speed") {
        const n = parseFloat(parts[1] ?? "");
        if (!isNaN(n) && n > 0) { config.speed = Math.max(0.5, Math.min(3.0, n)); ctx.ui.notify(`speed → ${config.speed}`, "info"); }
        else ctx.ui.notify("Usage: /tts speed <0.5-3.0>", "warning");
        return;
      }

      if (sub === "lang") {
        const a = parts[1] as "en" | "ru" | undefined;
        const b = parts[2] as "en" | "ru" | undefined;
        const valid = (x?: string): x is "en" | "ru" => x === "en" || x === "ru";
        if (!valid(a)) { ctx.ui.notify("Usage: /tts lang en|ru [en|ru]", "warning"); return; }
        config.inputLang = a; config.outputLang = valid(b) ? b : a;
        wsSend({ type: "config", key: "lang", value: `${config.inputLang},${config.outputLang}` });
        ctx.ui.notify(`language → ${config.inputLang.toUpperCase()}→${config.outputLang.toUpperCase()}`, "info");
        updateStatus(ctx);
        return;
      }

      if (sub === "mat") {
        const matSub = parts[1] ?? "";
        if (matSub === "on" || matSub === "off") { config.matEnabled = matSub === "on"; ctx.ui.notify(`Mat ${config.matEnabled ? "enabled" : "disabled"}`, "info"); updateStatus(ctx); return; }
        const prob = parseFloat(matSub);
        if (!isNaN(prob) && prob >= 0 && prob <= 1) { config.matProb = prob; ctx.ui.notify(`Mat prob → ${prob}`, "info"); return; }
        ctx.ui.notify(`Mat: ${config.matEnabled ? "on" : "off"} (prob=${config.matProb})`, "info");
        return;
      }

      if (sub === "interject") {
        const ijSub = parts[1] ?? "";
        if (ijSub === "on" || ijSub === "off") { config.interjectEnabled = ijSub === "on"; ctx.ui.notify(`Interject ${config.interjectEnabled ? "enabled" : "disabled"}`, "info"); updateStatus(ctx); return; }
        const prob = parseFloat(ijSub);
        if (!isNaN(prob) && prob >= 0 && prob <= 1) { config.interjectProb = prob; ctx.ui.notify(`Interject prob → ${prob}`, "info"); return; }
        ctx.ui.notify(`Interject: ${config.interjectEnabled ? "on" : "off"} (prob=${config.interjectProb})`, "info");
        return;
      }

      if (sub === "rvc") {
        const rvcSub = parts[1] ?? "";
        if (rvcSub === "on" || rvcSub === "off") { config.rvcEnabled = rvcSub === "on"; ctx.ui.notify(`RVC ${config.rvcEnabled ? "enabled" : "disabled"}`, "info"); updateStatus(ctx); return; }
        if (rvcSub === "url") { config.rvcUrl = parts[2] ?? config.rvcUrl; ctx.ui.notify(`RVC URL → ${config.rvcUrl}`, "info"); return; }
        if (rvcSub === "model") {
          if (!parts[2]) {
            try {
              const r = await fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(3000) });
              const models = ((await r.json()) as { models?: string[] }).models ?? [];
              const picked = await pickModel(ctx, models, config.rvcModel);
              if (picked) { config.rvcModel = picked; await fetch(`${config.rvcUrl}/models/${encodeURIComponent(picked)}`, { method: "POST", signal: AbortSignal.timeout(10_000) }); }
            } catch { ctx.ui.notify(`RVC unreachable at ${config.rvcUrl}`, "warning"); }
          } else { config.rvcModel = parts[2]; }
          updateStatus(ctx);
          return;
        }
        if (rvcSub === "models") {
          try {
            const r = await fetch(`${config.rvcUrl}/models`, { signal: AbortSignal.timeout(5_000) });
            const data = await r.json() as { models?: string[] };
            ctx.ui.notify((data.models ?? []).join("\n") || "No models", "info");
          } catch { ctx.ui.notify("RVC unreachable", "warning"); }
          return;
        }
        ctx.ui.notify("Usage: /tts rvc on|off|model|url|models", "warning");
        return;
      }

      if (sub === "stop") { wsSend({ type: "reset" }); ctx.ui.notify("TTS stopped", "info"); return; }
      if (sub === "mute") { muted = true; ctx.ui.notify("TTS muted", "info"); updateStatus(ctx); return; }
      if (sub === "unmute") { muted = false; ctx.ui.notify("TTS unmuted", "info"); updateStatus(ctx); return; }
      if (sub === "cache") { ctx.ui.notify("/tts cache — managed by Rust engine", "info"); return; }

      if (sub === "mix") {
        const mixSub = parts[1] ?? "";
        if (mixSub === "status") { await loadMixerSession(); ctx.ui.notify(mixerContext(), "info"); updateStatus(ctx); return; }
        if (mixSub === "apply") {
          const w = mixerWinner();
          ctx.ui.notify(w ? `Winner: ${w.label}\n${JSON.stringify(w.opts, null, 2)}` : "No winner set", w ? "info" : "warning");
          return;
        }
        ctx.ui.notify("Usage: /tts mix status | apply", "info");
        return;
      }

      if (sub === "") {
        config.enabled = !config.enabled;
        updateStatus(ctx);
        return;
      }

      await openFoniPanel(ctx, panelState(), panelActions(ctx));
      updateStatus(ctx);
    },
  });
}
