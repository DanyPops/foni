/**
 * Foni interactive panel — triggered by /tts with no arguments.
 *
 * ╭─────────── 🔊 Foni ─────────────╮
 * │                                   │
 * │  ▶ TTS        [ON ] espeak        │
 * │    Language   [RU ] 🇷🇺           │
 * │    Speed      [1.0×]              │
 * │    Backend    [auto]              │
 * │                                   │
 * │  ─── RVC ─────────────────────   │
 * │                                   │
 * │    Voice      [ON ] bandit        │
 * │    Server     [OK ] :5050         │
 * │                                   │
 * ├───────────────────────────────────┤
 * │  ↑↓ navigate · space toggle · esc│
 * ╰───────────────────────────────────╯
 *
 * Keys: ↑↓ navigate · space/enter activate · + - speed
 *        l  language · b backend · m model picker · esc close
 */

import { matchesKey, truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

export interface FoniPanelState {
  enabled: boolean;
  inputLang:  "en" | "ru";
  outputLang: "en" | "ru";
  speed: number;
  backendName: string;
  backendPref: string;
  rvcEnabled: boolean;
  rvcModel: string;
  rvcUrl: string;
  rvcServerOk: boolean | null;
}

export interface FoniPanelActions {
  toggle(): void;
  setLang(inputLang: "en" | "ru", outputLang: "en" | "ru"): void;
  setSpeed(speed: number): void;
  setBackendPref(pref: string): void;
  toggleRvc(): void;
  pickRvcModel(): Promise<void>;
  checkRvcServer(): Promise<boolean>;
}

const BACKENDS = ["auto", "espeak", "say"] as const;

const PANEL_WIDTH = 52;

class FoniPanel {
  private cursor = 0;
  private cachedLines?: string[];
  private cachedWidth?: number;

  onClose?: () => void;
  private invalidateFn?: () => void;

  private readonly rows = [
    "tts", "lang", "speed", "backend", "_divider_", "rvc", "model", "server",
  ] as const;

  constructor(
    private state: FoniPanelState,
    private actions: FoniPanelActions,
    invalidate: () => void,
  ) {
    this.invalidateFn = invalidate;
    this.cursor = 0;
  }

  invalidate(): void {
    this.cachedLines = undefined;
    this.cachedWidth = undefined;
    this.invalidateFn?.();
  }

  updateState(state: FoniPanelState): void {
    this.state = state;
    this.invalidate();
  }

  private navigableRows(): number[] {
    return this.rows.map((r, i) => [r, i] as const)
      .filter(([r]) => r !== "_divider_")
      .map(([, i]) => i);
  }

  private moveCursor(dir: 1 | -1): void {
    const nav = this.navigableRows();
    const pos = nav.indexOf(this.cursor);
    const next = nav[Math.max(0, Math.min(nav.length - 1, pos + dir))];
    if (next !== undefined) { this.cursor = next; this.invalidate(); }
  }

  async handleInput(data: string): Promise<void> {
    if (matchesKey(data, "escape") || matchesKey(data, "ctrl+c") || data === "q") {
      this.onClose?.();
      return;
    }
    if (matchesKey(data, "up")) { this.moveCursor(-1); return; }
    if (matchesKey(data, "down")) { this.moveCursor(1); return; }

    const row = this.rows[this.cursor];

    // Global shortcuts
    if (data === "+") { this.actions.setSpeed(Math.min(3.0, +(this.state.speed + 0.1).toFixed(1))); this.invalidate(); return; }
    if (data === "-") { this.actions.setSpeed(Math.max(0.5, +(this.state.speed - 0.1).toFixed(1))); this.invalidate(); return; }
    if (data === "l") {
      // Cycle: en→en ► ru→ru ► en→ru
      const { inputLang, outputLang } = this.state;
      const next: ["en" | "ru", "en" | "ru"] =
        inputLang === "en" && outputLang === "en" ? ["ru", "ru"] :
        inputLang === "ru" && outputLang === "ru" ? ["en", "ru"] :
        ["en", "en"];
      this.actions.setLang(...next);
      this.invalidate();
      return;
    }
    if (data === "r") { this.actions.toggleRvc(); this.invalidate(); return; }
    if (data === "b") {
      const i = BACKENDS.indexOf(this.state.backendPref as typeof BACKENDS[number]);
      this.actions.setBackendPref(BACKENDS[(i + 1) % BACKENDS.length]);
      this.invalidate(); return;
    }
    if (data === "m") { await this.actions.pickRvcModel(); this.invalidate(); return; }

    // Row activation
    if (matchesKey(data, "space") || matchesKey(data, "return") || matchesKey(data, "enter")) {
      if (row === "tts")    { this.actions.toggle(); this.invalidate(); }
      if (row === "lang")   {
        const { inputLang, outputLang } = this.state;
        const next: ["en" | "ru", "en" | "ru"] =
          inputLang === "en" && outputLang === "en" ? ["ru", "ru"] :
          inputLang === "ru" && outputLang === "ru" ? ["en", "ru"] :
          ["en", "en"];
        this.actions.setLang(...next);
        this.invalidate();
      }
      if (row === "speed")  { /* hint shown */ }
      if (row === "backend"){ const i = BACKENDS.indexOf(this.state.backendPref as typeof BACKENDS[number]); this.actions.setBackendPref(BACKENDS[(i + 1) % BACKENDS.length]); this.invalidate(); }
      if (row === "rvc")    { this.actions.toggleRvc(); this.invalidate(); }
      if (row === "model")  { await this.actions.pickRvcModel(); this.invalidate(); }
      if (row === "server") { const ok = await this.actions.checkRvcServer(); this.state.rvcServerOk = ok; this.invalidate(); }
    }
  }

  render(width: number): string[] {
    if (this.cachedLines && this.cachedWidth === width) return this.cachedLines;

    const W = Math.min(width, PANEL_WIDTH);
    const innerW = W - 2;
    const s = this.state;

    // ── color helpers ─────────────────────────────────────────────────────────
    const fg = (code: string, t: string) => `\x1b[${code}m${t}\x1b[0m`;
    const border  = (t: string) => fg("2", t);
    const accent  = (t: string) => fg("36;1", t);
    const dim     = (t: string) => fg("2", t);
    const bold    = (t: string) => fg("1", t);
    const italic  = (t: string) => fg("3", t);
    const success = (t: string) => fg("32", t);
    const error   = (t: string) => fg("31", t);
    const warn    = (t: string) => fg("33", t);
    const sel     = (t: string) => fg("36", t);      // selected row text

    const badge = (on: boolean, label: string) =>
      on ? fg("32", `[${label}]`) : fg("2", `[${label}]`);

    // ── layout helpers ────────────────────────────────────────────────────────
    const row = (content: string) => {
      const padded = truncateToWidth(" " + content, innerW, "…", true);
      return border("│") + padded + border("│");
    };
    const empty = () => border("│") + " ".repeat(innerW) + border("│");
    const divider = () => border("├" + "─".repeat(innerW) + "┤");
    const cursor = (i: number) => this.cursor === i ? sel("▸") : " ";

    // ── header ────────────────────────────────────────────────────────────────
    const title = " Foni ";
    const bLen = innerW - visibleWidth(title);
    const lB = Math.floor(bLen / 2);
    const rB = bLen - lB;
    const lines: string[] = [];
    lines.push(border("╭" + "─".repeat(lB)) + bold(title) + border("─".repeat(rB) + "╮"));
    lines.push(empty());

    // ── row renderer ─────────────────────────────────────────────────────────
    const makeRow = (idx: number, label: string, valueStr: string) => {
      const isActive = this.cursor === idx;
      const c = cursor(idx);
      const lbl = (isActive ? sel : dim)(label.padEnd(10));
      const val = isActive ? accent(valueStr) : valueStr;
      return row(`${c} ${lbl} ${val}`);
    };

    // 0: TTS on/off
    lines.push(makeRow(0, "TTS",
      badge(s.enabled, s.enabled ? " ON" : "OFF") + " " + (s.enabled ? accent(s.backendName) : dim("-"))));  

    // 1: Language
    const langStr = s.inputLang === s.outputLang
      ? s.outputLang.toUpperCase()
      : `${s.inputLang.toUpperCase()}→${s.outputLang.toUpperCase()}`;
    lines.push(makeRow(1, "Language",
      badge(s.outputLang === "ru", langStr)));

    // 2: Speed
    lines.push(makeRow(2, "Speed",
      accent(`${s.speed.toFixed(1)}x`) + dim("  +/- to adjust")));

    // 3: Backend
    lines.push(makeRow(3, "Backend",
      dim("[") + accent(s.backendPref) + dim("] ") + dim(italic("space to cycle"))));

    lines.push(empty());

    // divider
    const rvcTitle = " RVC Voice ";
    const rvcBLen = innerW - visibleWidth(rvcTitle);
    lines.push(border("│") + dim("─".repeat(Math.floor(rvcBLen / 2))) + dim(rvcTitle) + dim("─".repeat(rvcBLen - Math.floor(rvcBLen / 2))) + border("│"));
    lines.push(empty());

    // 5: RVC on/off  (index 5 because _divider_ is 4 — but rows[] maps it as index 5)
    lines.push(makeRow(5, "Voice conv",
      badge(s.rvcEnabled, s.rvcEnabled ? " ON" : "OFF") + " " + (s.rvcEnabled ? accent(s.rvcModel || "-") : dim("-"))));  

    // 6: Model
    lines.push(makeRow(6, "Model",
      s.rvcModel ? accent(s.rvcModel) : warn("none -- press m to pick")));

    // 7: Server
    const serverStatus = s.rvcServerOk === null
      ? dim("unknown -- press Enter to check")
      : s.rvcServerOk ? success("● online") : error("○ unreachable");
    lines.push(makeRow(7, "Server",
      dim(s.rvcUrl + "  ") + serverStatus));

    lines.push(empty());
    lines.push(divider());
    lines.push(empty());

    // hints
    const hints = [
      italic("↑↓") + " move",
      italic("space") + " toggle",
      italic("m") + " model",
      italic("+/-") + " speed",
      italic("esc") + " close",
    ].map(h => dim(h)).join(dim("  ·  "));
    lines.push(row(hints));

    lines.push(empty());
    lines.push(border("╰" + "─".repeat(innerW) + "╯"));

    this.cachedLines = lines;
    this.cachedWidth = width;
    return lines;
  }
}

export async function openFoniPanel(
  ctx: ExtensionContext,
  state: FoniPanelState,
  actions: FoniPanelActions,
): Promise<void> {
  return new Promise<void>((resolve) => {
    ctx.ui.custom(
      (tui, _theme, _keybindings, done) => {
        const panel = new FoniPanel(state, actions, () => tui.requestRender());
        panel.onClose = () => { done(undefined); resolve(); };

        const proxy = {
          render: (w: number) => panel.render(w),
          handleInput: async (data: string) => {
            await panel.handleInput(data);
            if (!panel.onClose) { done(undefined); resolve(); }
          },
          invalidate: () => panel.invalidate(),
        };
        return proxy;
      },
      { overlay: true, overlayOptions: { anchor: "center", width: PANEL_WIDTH } },
    );
  });
}
