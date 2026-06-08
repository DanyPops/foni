/**
 * Foni TTS panel — opened by /tts with no arguments.
 *
 * ╭──────────── 🔊 Foni ─────────────────╮
 * │                                        │
 * │  ▶ TTS          [ON ]  synth           │
 * │    Voice        [sid]  sidorovich       │
 * │    Language     [EN→RU]                 │
 * │    Translation  [NLLB]                  │
 * │                                        │
 * │  ─── Personality ─────────────────── │
 * │    Mat          [ON ]  0.35            │
 * │    Interject    [ON ]  0.25            │
 * │    Stress       [DICT]                 │
 * │                                        │
 * ├────────────────────────────────────────┤
 * │  ↑↓ navigate  space toggle  esc close  │
 * ╰────────────────────────────────────────╯
 */

import { matchesKey, Key, visibleWidth } from "@earendil-works/pi-tui";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

export interface FoniPanelState {
  enabled: boolean;
  muted: boolean;
  wsConnected: boolean;
  voice: string;
  inputLang: string;
  outputLang: string;
  matEnabled: boolean;
  matProb: number;
  interjectEnabled: boolean;
  interjectProb: number;
}

export interface FoniPanelActions {
  toggleEnabled(): void;
  toggleMuted(): void;
  toggleMat(): void;
  toggleInterject(): void;
  stop(): void;
  test(): Promise<void>;
}

const PANEL_WIDTH = 54;

function fg(code: string, text: string): string {
  return `\x1b[${code}m${text}\x1b[0m`;
}
function bold(s: string): string { return `\x1b[1m${s}\x1b[22m`; }
function dim(s: string): string  { return `\x1b[2m${s}\x1b[22m`; }
function inv(s: string): string  { return `\x1b[7m${s}\x1b[27m`; }

const inner = PANEL_WIDTH - 2;  // inside the border

function row(content: string): string {
  const w = visibleWidth(content);
  const pad = Math.max(0, inner - w);
  return fg("2", "│") + " " + content + " ".repeat(pad - 1) + fg("2", "│");
}
function emptyRow(): string { return row(""); }
function divider(): string  { return fg("2", "├" + "─".repeat(inner) + "┤"); }

const ITEMS = [
  { id: "enabled",    label: "TTS" },
  { id: "muted",      label: "Mute" },
  { id: "mat",        label: "Mat" },
  { id: "interject",  label: "Interject" },
  { id: "stop",       label: "Stop stream" },
  { id: "test",       label: "Ping foni-synth" },
] as const;

type ItemId = typeof ITEMS[number]["id"];

class FoniPanel {
  private cursor = 0;
  private cachedLines?: string[];
  onClose?: () => void;
  private invalidateFn?: () => void;

  constructor(
    private state: FoniPanelState,
    private actions: FoniPanelActions,
  ) {}

  setInvalidate(fn: () => void): void { this.invalidateFn = fn; }

  invalidate(): void {
    this.cachedLines = undefined;
    this.invalidateFn?.();
  }

  render(_width: number): string[] {
    if (this.cachedLines) return this.cachedLines;

    const lines: string[] = [];
    const s = this.state;

    const title = bold("🔊 Foni");
    const titleW = visibleWidth("🔊 Foni");
    const lPad = Math.floor((inner - titleW) / 2);
    const rPad = inner - titleW - lPad;
    lines.push(fg("2", "╭" + "─".repeat(lPad - 1)) + " " + title + " " + fg("2", "─".repeat(rPad - 1) + "╮"));

    lines.push(emptyRow());

    const items = ITEMS;
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      const isCursor = i === this.cursor;
      const prefix = isCursor ? fg("36", "▶") : dim(" ");

      let valueStr = "";
      switch (item.id) {
        case "enabled":
          valueStr = s.enabled ? fg("32", "ON ") : fg("31", "OFF");
          break;
        case "muted":
          valueStr = s.muted ? fg("33", "🔇 ") : dim("---");
          break;
        case "mat":
          valueStr = s.matEnabled ? fg("32", `ON  ${dim(s.matProb.toFixed(2))}`) : dim("off");
          break;
        case "interject":
          valueStr = s.interjectEnabled ? fg("32", `ON  ${dim(s.interjectProb.toFixed(2))}`) : dim("off");
          break;
        case "stop":
        case "test":
          valueStr = isCursor ? fg("36", "⏎ run") : dim("action");
          break;
      }

      if (item.id === "stop" && i > 0) {
        lines.push(emptyRow());
        lines.push(row(dim("─── Actions " + "─".repeat(Math.max(0, inner - 15)))));
        lines.push(emptyRow());
      }

      const label = bold(item.label.padEnd(12));
      const rowContent = `${prefix} ${label} [${valueStr}]`;
      lines.push(row(isCursor ? rowContent : rowContent));
    }

    lines.push(emptyRow());

    // Info section
    const lang = s.inputLang === s.outputLang
      ? s.outputLang.toUpperCase()
      : `${s.inputLang.toUpperCase()}→${s.outputLang.toUpperCase()}`;
    lines.push(row(dim(`voice: ${s.voice || "(none)"}   lang: ${lang}`)));
    lines.push(row(dim(`ws: ${s.wsConnected ? fg("32", "connected") : fg("31", "disconnected")}`)));

    lines.push(emptyRow());
    lines.push(divider());
    lines.push(emptyRow());
    lines.push(row(dim("↑↓ navigate  space/⏎ toggle  esc close")));
    lines.push(emptyRow());
    lines.push(fg("2", "╰" + "─".repeat(inner) + "╯"));

    this.cachedLines = lines;
    return lines;
  }

  async handleInputAsync(data: string): Promise<{ close?: boolean }> {
    const items = ITEMS;
    const item = items[this.cursor];

    if (matchesKey(data, Key.up) || data === "k") {
      this.cursor = (this.cursor - 1 + items.length) % items.length;
      this.invalidate();
      return {};
    }
    if (matchesKey(data, Key.down) || data === "j") {
      this.cursor = (this.cursor + 1) % items.length;
      this.invalidate();
      return {};
    }
    if (matchesKey(data, Key.escape) || data === "q") {
      return { close: true };
    }
    if (data === " " || matchesKey(data, Key.enter)) {
      await this.activate(item.id);
      return {};
    }

    return {};
  }

  private async activate(id: ItemId): Promise<void> {
    switch (id) {
      case "enabled":   this.actions.toggleEnabled();   break;
      case "muted":     this.actions.toggleMuted();     break;
      case "mat":       this.actions.toggleMat();       break;
      case "interject": this.actions.toggleInterject(); break;
      case "stop":      this.actions.stop();            break;
      case "test":      await this.actions.test();      break;
    }
    this.invalidate();
  }

  updateState(next: Partial<FoniPanelState>): void {
    Object.assign(this.state, next);
    this.invalidate();
  }
}

export function openFoniPanel(
  ctx: ExtensionContext,
  state: FoniPanelState,
  actions: FoniPanelActions,
): void {
  if (!ctx.hasUI) return;

  ctx.ui.custom(
    (_tui, _theme, _keybindings, done) => {
      const panel = new FoniPanel(state, actions);

      const component = {
        render(width: number): string[] { return panel.render(width); },
        handleInput(data: string): void {
          void panel.handleInputAsync(data).then((result) => {
            if (result.close) done(undefined);
          });
        },
        invalidate(): void { panel.invalidate(); },
        dispose() {},
      };

      return component;
    },
    { overlay: true, overlayOptions: { anchor: "center", width: PANEL_WIDTH } },
  );
}
