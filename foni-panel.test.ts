import { describe, it, expect, vi, beforeEach } from "vitest";



vi.mock("@earendil-works/pi-tui", () => {
  const KEYS: Record<string, string> = {
    escape:    "\x1b",
    up:        "\x1b[A",
    down:      "\x1b[B",
    return:    "\r",
    enter:     "\r",
    space:     " ",
    backspace: "\x7f",
    "ctrl+c":  "\x03",
  };
  return {
    matchesKey:      (data: string, key: string) => data === (KEYS[key] ?? key),
    truncateToWidth: (str: string, width: number, _e = "…", pad = false) => {
      const vis = str.replace(/\x1b\[[^m]*m/g, "");
      const out = vis.length <= width ? str : str.slice(0, width) + _e;
      return pad ? out.padEnd(width) : out;
    },
    visibleWidth: (str: string) => str.replace(/\x1b\[[^m]*m/g, "").length,
  };
});

import { openFoniPanel } from "./tui/foni-panel.ts";
import type { FoniPanelState, FoniPanelActions } from "./tui/foni-panel.ts";
function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[^m]*m/g, "");
}

function makeState(overrides: Partial<FoniPanelState> = {}): FoniPanelState {
  return {
    enabled:     false,
    inputLang:   "en",
    outputLang:  "en",
    speed:       1.0,
    backendName: "espeak",
    backendPref: "auto",
    rvcEnabled:  false,
    rvcModel:    "sidorovich",
    rvcUrl:      "http://localhost:5050",
    rvcServerOk: null,
    ...overrides,
  };
}

function makeActions(overrides: Partial<FoniPanelActions> = {}): FoniPanelActions {
  return {
    toggle:         vi.fn(),
    setLang:        vi.fn(),
    setSpeed:       vi.fn(),
    setBackendPref: vi.fn(),
    toggleRvc:      vi.fn(),
    pickRvcModel:   vi.fn().mockResolvedValue(undefined),
    checkRvcServer: vi.fn().mockResolvedValue(true),
    ...overrides,
  };
}

async function mountPanel(state = makeState(), actions = makeActions()) {
  let component!: { render(w: number): string[]; handleInput(d: string): Promise<void> };
  let panelResolve!: () => void;
  const panelPromise = new Promise<void>(r => { panelResolve = r; });

  const ctx = {
    ui: {
      custom: vi.fn((factory: Function) => {
        const tui  = { requestRender: vi.fn() };
        const done = vi.fn(panelResolve);
        component = factory(tui, null, null, done);
      }),
    },
  } as any;

  openFoniPanel(ctx, state, actions);
  if (!component) throw new Error("factory not called synchronously");

  /** Render as plain text the way a human reads it. */
  const screen = (width = 52) =>
    component.render(width).map(stripAnsi).join("\n");

  return { component, panelPromise, screen, actions };
}


describe("FoniPanel render snapshots", () => {
  it("initial state (TTS off, RVC off, EN)", async () => {
    const { screen } = await mountPanel();
    expect(screen()).toMatchSnapshot();
  });

  it("TTS enabled", async () => {
    const { screen } = await mountPanel(makeState({ enabled: true }));
    expect(screen()).toMatchSnapshot();
  });

  it("RVC enabled with sidorovich model", async () => {
    const { screen } = await mountPanel(makeState({ rvcEnabled: true, rvcModel: "sidorovich" }));
    expect(screen()).toMatchSnapshot();
  });

  it("language RU", async () => {
    const { screen } = await mountPanel(makeState({ inputLang: "ru", outputLang: "ru" }));
    expect(screen()).toMatchSnapshot();
  });

  it("speed 1.5x", async () => {
    const { screen } = await mountPanel(makeState({ speed: 1.5 }));
    expect(screen()).toMatchSnapshot();
  });

  it("narrow terminal (40 cols)", async () => {
    const { screen } = await mountPanel();
    expect(screen(40)).toMatchSnapshot();
  });
});

//
// Feed a keystroke, assert the right action was called with the right args.
// We test what the panel *does*, not what it *looks like* after.

describe("FoniPanel handleInput → actions", () => {
  it("ESC resolves the panel promise", async () => {
    const { component, panelPromise } = await mountPanel();
    await component.handleInput("\x1b");
    await expect(panelPromise).resolves.toBeUndefined();
  });

  it("q resolves the panel promise", async () => {
    const { component, panelPromise } = await mountPanel();
    await component.handleInput("q");
    await expect(panelPromise).resolves.toBeUndefined();
  });

  it("space on TTS row (cursor=0) calls toggle()", async () => {
    const { component, actions } = await mountPanel();
    await component.handleInput(" ");
    expect(actions.toggle).toHaveBeenCalledOnce();
  });

  it("+ calls setSpeed with speed+0.1", async () => {
    const { component, actions } = await mountPanel(makeState({ speed: 1.0 }));
    await component.handleInput("+");
    expect(actions.setSpeed).toHaveBeenCalledWith(1.1);
  });

  it("- calls setSpeed with speed-0.1", async () => {
    const { component, actions } = await mountPanel(makeState({ speed: 1.0 }));
    await component.handleInput("-");
    expect(actions.setSpeed).toHaveBeenCalledWith(0.9);
  });

  it("+ clamps at 3.0", async () => {
    const { component, actions } = await mountPanel(makeState({ speed: 3.0 }));
    await component.handleInput("+");
    expect(actions.setSpeed).toHaveBeenCalledWith(3.0);
  });

  it("- clamps at 0.5", async () => {
    const { component, actions } = await mountPanel(makeState({ speed: 0.5 }));
    await component.handleInput("-");
    expect(actions.setSpeed).toHaveBeenCalledWith(0.5);
  });

  it("l toggles lang en→ru", async () => {
    const { component, actions } = await mountPanel(makeState({ inputLang: "en", outputLang: "en" }));
    await component.handleInput("l");
    expect(actions.setLang).toHaveBeenCalledWith("ru", "ru");
  });

  it("l toggles lang ru→en", async () => {
    const { component, actions } = await mountPanel(makeState({ inputLang: "ru", outputLang: "ru" }));
    await component.handleInput("l");
    expect(actions.setLang).toHaveBeenCalledWith("en", "ru");
  });

  it("r calls toggleRvc", async () => {
    const { component, actions } = await mountPanel();
    await component.handleInput("r");
    expect(actions.toggleRvc).toHaveBeenCalledOnce();
  });

  it("m calls pickRvcModel", async () => {
    const { component, actions } = await mountPanel();
    await component.handleInput("m");
    expect(actions.pickRvcModel).toHaveBeenCalledOnce();
  });

  it("b cycles backend auto→espeak", async () => {
    const { component, actions } = await mountPanel(makeState({ backendPref: "auto" }));
    await component.handleInput("b");
    expect(actions.setBackendPref).toHaveBeenCalledWith("espeak");
  });

  it("↓ then space activates the language row", async () => {
    const { component, actions } = await mountPanel(makeState({ inputLang: "en", outputLang: "en" }));
    await component.handleInput("\x1b[B"); // down → lang row
    await component.handleInput(" ");      // space → activate
    expect(actions.setLang).toHaveBeenCalledWith("ru", "ru");
  });
});

//
// The panel receives a one-time state snapshot on mount.
// Actions mutate external config but panel.updateState() is never called,
// so rendered badges go stale after an action fires.
// This test pins the bug so it's visible when fixed.

describe("stale snapshot (known bug)", () => {
  it("render does not reflect toggle() fired during the session", async () => {
    let externalEnabled = false;
    const actions = makeActions({
      toggle: vi.fn(() => { externalEnabled = !externalEnabled; }),
    });

    const { component, screen } = await mountPanel(makeState({ enabled: false }), actions);
    await component.handleInput(" "); // fires toggle()

    expect(externalEnabled).toBe(true);            // external state updated ✓
    expect(screen()).toContain("[OFF]");            // panel still stale ✗ (the bug)
  });
});
