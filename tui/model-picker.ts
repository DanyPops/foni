/**
 * ModelPicker — interactive RVC model selector overlay.
 *
 * Usage:
 *   const model = await pickModel(ctx, models, currentModel);
 *   if (model) { ... load model ... }
 */

import { matchesKey, truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

class ModelPickerComponent {
  private selected: number;
  private cachedLines?: string[];
  private cachedWidth?: number;
  public invalidate: () => void;

  onConfirm?: (model: string) => void;
  onCancel?: () => void;

  constructor(
    private readonly models: string[],
    private readonly current: string,
    invalidate: () => void,
  ) {
    this.selected = Math.max(0, models.indexOf(current));
    this.invalidate = invalidate;
  }

  handleInput(data: string): void {
    if (matchesKey(data, "up") && this.selected > 0) {
      this.selected--;
      this.invalidate();
    } else if (matchesKey(data, "down") && this.selected < this.models.length - 1) {
      this.selected++;
      this.invalidate();
    } else if (matchesKey(data, "return") || matchesKey(data, "enter")) {
      this.onConfirm?.(this.models[this.selected]);
    } else if (matchesKey(data, "escape") || matchesKey(data, "ctrl+c")) {
      this.onCancel?.();
    }
  }

  render(width: number): string[] {
    if (this.cachedLines && this.cachedWidth === width) return this.cachedLines;

    const title = " Select RVC model ";
    const hint = " ↑↓ move · Enter confirm · Esc cancel ";
    const inner = width - 4;

    const border = (s: string) => `\x1b[2m${s}\x1b[0m`;
    const accent = (s: string) => `\x1b[36m${s}\x1b[0m`;
    const dim = (s: string) => `\x1b[2m${s}\x1b[0m`;

    const lines: string[] = [];
    lines.push(border(`┌─${title}${"─".repeat(Math.max(0, width - 2 - visibleWidth(title)))}┐`));

    if (this.models.length === 0) {
      lines.push(border("│") + dim("  No models found in rvc_models/".padEnd(width - 2)) + border("│"));
    } else {
      for (let i = 0; i < this.models.length; i++) {
        const selected = i === this.selected;
        const current  = this.models[i] === this.current;
        const prefix   = selected ? "  > " : "    ";
        const suffix   = current ? " *" : "  ";
        const label    = truncateToWidth(prefix + this.models[i] + suffix, inner);
        const padded   = label.padEnd(inner + (label.length - visibleWidth(label)));
        const row = border("│ ") + (selected ? accent(padded) : padded) + border(" │");
        lines.push(row);
      }
    }

    lines.push(border(`└─${hint}${"─".repeat(Math.max(0, width - 2 - visibleWidth(hint)))}┘`));

    this.cachedLines = lines;
    this.cachedWidth = width;
    return lines;
  }
}

export async function pickModel(
  ctx: ExtensionContext,
  models: string[],
  current: string,
): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    ctx.ui.custom(
      (tui, _theme, _keybindings, done) => {
        const picker = new ModelPickerComponent(models, current, () => tui.requestRender());
        picker.onConfirm = (m) => { done(undefined); resolve(m); };
        picker.onCancel  = ()  => { done(undefined); resolve(null); };
        return picker;
      },
      { overlay: true, overlayOptions: { anchor: "center", width: 44 } },
    );
  });
}
