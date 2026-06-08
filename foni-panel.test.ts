import { describe, it, expect, vi } from "vitest";

vi.mock("@earendil-works/pi-tui", () => ({
  matchesKey: (data: string, key: unknown) => {
    const MAP: Record<string, string> = {
      up: "\x1b[A", down: "\x1b[B", escape: "\x1b", enter: "\r",
    };
    const keyStr = typeof key === "string" ? key : JSON.stringify(key);
    return data === (MAP[keyStr] ?? keyStr);
  },
  Key: { up: "up", down: "down", escape: "escape", enter: "enter" },
  visibleWidth: (s: string) => s.replace(/\x1b\[[^m]*m/g, "").length,
}));

import { openFoniPanel } from "./tui/foni-panel.ts";
import type { FoniPanelState, FoniPanelActions } from "./tui/foni-panel.ts";

function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[^m]*m/g, "");
}

function makeState(o: Partial<FoniPanelState> = {}): FoniPanelState {
  return {
    enabled: false, muted: false, wsConnected: true,
    voice: "sidorovich", inputLang: "en", outputLang: "ru",
    matEnabled: true, matProb: 0.35,
    interjectEnabled: true, interjectProb: 0.25,
    ...o,
  };
}

function makeActions(o: Partial<FoniPanelActions> = {}): FoniPanelActions {
  return {
    toggleEnabled:   vi.fn(),
    toggleMuted:     vi.fn(),
    toggleMat:       vi.fn(),
    toggleInterject: vi.fn(),
    stop:            vi.fn(),
    test:            vi.fn().mockResolvedValue(undefined),
    ...o,
  };
}

function capturePanel(state: FoniPanelState, actions: FoniPanelActions) {
  let capturedFactory: ((tui: unknown, theme: unknown, kb: unknown, done: (r: unknown) => void) => unknown) | null = null;
  const ctx = {
    hasUI: true,
    ui: {
      custom: vi.fn((factory, _opts) => { capturedFactory = factory; }),
    },
  } as unknown as Parameters<typeof openFoniPanel>[0];

  openFoniPanel(ctx, state, actions);

  expect(capturedFactory).not.toBeNull();

  let doneValue: unknown;
  const done = (v: unknown) => { doneValue = v; };
  const comp = capturedFactory!(null, null, null, done) as {
    render(w: number): string[];
    handleInput(d: string): void;
    invalidate(): void;
  };

  return { comp, getDone: () => doneValue };
}

describe("FoniPanel render", () => {
  it("renders border and title", () => {
    const { comp } = capturePanel(makeState(), makeActions());
    const lines = comp.render(54).map(stripAnsi);
    expect(lines[0]).toContain("Foni");
    expect(lines[0]).toContain("╭");
    expect(lines[lines.length - 1]).toContain("╯");
  });

  it("shows ON when enabled", () => {
    const { comp } = capturePanel(makeState({ enabled: true }), makeActions());
    const text = comp.render(54).map(stripAnsi).join("\n");
    expect(text).toContain("ON");
  });

  it("shows OFF when disabled", () => {
    const { comp } = capturePanel(makeState({ enabled: false }), makeActions());
    const text = comp.render(54).map(stripAnsi).join("\n");
    expect(text).toContain("OFF");
  });

  it("shows voice and lang in info section", () => {
    const { comp } = capturePanel(makeState({ voice: "diomedes" }), makeActions());
    const text = comp.render(54).map(stripAnsi).join("\n");
    expect(text).toContain("diomedes");
    expect(text).toContain("EN→RU");
  });

  it("shows connected ws status", () => {
    const { comp } = capturePanel(makeState({ wsConnected: true }), makeActions());
    const text = comp.render(54).map(stripAnsi).join("\n");
    expect(text).toContain("connected");
  });

  it("renders navigation hint", () => {
    const { comp } = capturePanel(makeState(), makeActions());
    const text = comp.render(54).map(stripAnsi).join("\n");
    expect(text).toContain("navigate");
    expect(text).toContain("esc");
  });
});

describe("FoniPanel keyboard", () => {
  it("esc closes the panel", async () => {
    const { comp, getDone } = capturePanel(makeState(), makeActions());
    comp.handleInput("\x1b");
    await new Promise(r => setTimeout(r, 10));
    expect(getDone()).toBeUndefined(); // done called with undefined
  });

  it("q closes the panel", async () => {
    const { comp, getDone } = capturePanel(makeState(), makeActions());
    comp.handleInput("q");
    await new Promise(r => setTimeout(r, 10));
    expect(getDone()).toBeUndefined();
  });

  it("down moves cursor", () => {
    const { comp } = capturePanel(makeState(), makeActions());
    const before = comp.render(54).map(stripAnsi).join("\n");
    comp.handleInput("\x1b[B"); // down
    const after = comp.render(54).map(stripAnsi).join("\n");
    expect(before).not.toEqual(after); // cursor changed
  });

  it("space on TTS row calls toggleEnabled", async () => {
    const actions = makeActions();
    const { comp } = capturePanel(makeState(), actions);
    // Cursor starts at 0 = TTS/enabled row
    comp.handleInput(" ");
    await new Promise(r => setTimeout(r, 10));
    expect(actions.toggleEnabled).toHaveBeenCalled();
  });

  it("space on mute row calls toggleMuted", async () => {
    const actions = makeActions();
    const { comp } = capturePanel(makeState(), actions);
    comp.handleInput("\x1b[B"); // down to mute
    comp.handleInput(" ");
    await new Promise(r => setTimeout(r, 10));
    expect(actions.toggleMuted).toHaveBeenCalled();
  });

  it("invalidate clears cached render", () => {
    const { comp } = capturePanel(makeState(), makeActions());
    comp.render(54); // populate cache
    comp.invalidate();
    // Should re-render without throwing
    expect(() => comp.render(54)).not.toThrow();
  });
});

describe("openFoniPanel no-op when no UI", () => {
  it("does not call custom when hasUI=false", () => {
    const ctx = {
      hasUI: false,
      ui: { custom: vi.fn() },
    } as unknown as Parameters<typeof openFoniPanel>[0];
    openFoniPanel(ctx, makeState(), makeActions());
    expect(ctx.ui.custom).not.toHaveBeenCalled();
  });
});
