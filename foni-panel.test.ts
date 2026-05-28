import { describe, it, expect, vi, beforeEach } from "vitest";

// ─── Mock @earendil-works/pi-tui ─────────────────────────────────────────────

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
import { MatTranslator, InterjectTranslator, stretchExpression } from "./pipeline/translators.ts";

// ─── Helpers ─────────────────────────────────────────────────────────────────

/** Strip all ANSI escape codes — what a human sees in the terminal. */
function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[^m]*m/g, "");
}

function makeState(overrides: Partial<FoniPanelState> = {}): FoniPanelState {
  return {
    enabled:     false,
    lang:        "en",
    speed:       1.0,
    backendName: "silero",
    backendPref: "auto",
    rvcEnabled:  false,
    rvcModel:    "bandit",
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

// ─── MatTranslator ───────────────────────────────────────────────────────────

describe("MatTranslator", () => {
  it("prob=0 is a pure passthrough", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Привет мир.") };
    const t = new MatTranslator(inner, 0);
    expect(await t.translate("Hello world")).toBe("Привет мир.");
  });

  it("always delegates to inner first", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Привет.") };
    await new MatTranslator(inner, 0).translate("Hello");
    expect(inner.translate).toHaveBeenCalledWith("Hello");
  });

  it("prob=1 injects mat at every comma gap", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Один, два, три.") };
    const result = await new MatTranslator(inner, 1).translate("One, two, three.");
    const mat = ["блядь","сука","хуй","пиздец","ёпта","блин","ёб твою мать","нихуя себе"];
    expect(mat.some(w => result.includes(w))).toBe(true);
  });

  it("probability is public and mutable", () => {
    const t = new MatTranslator({ translate: vi.fn() }, 0.35);
    t.probability = 0.8;
    expect(t.probability).toBe(0.8);
  });

  it("stretchProbability defaults to 0.5", () => {
    const t = new MatTranslator({ translate: vi.fn() });
    expect(t.stretchProbability).toBe(0.5);
  });

  it("stretchProbability=0 never alters injected words (no repeated vowels)", async () => {
    // With prob=1 mat is always injected; stretchProb=0 means never stretched.
    // Stretched words contain repeated vowels (e.g. "бляяядь") — none should appear.
    const inner = { translate: vi.fn().mockResolvedValue("Один, два, три.") };
    const t = new MatTranslator(inner, 1, 0);
    const result = await t.translate("x");
    // No Russian vowel should be repeated 3+ times consecutively
    expect(result).not.toMatch(/[АаОоУуЕеЁёИиЭэЮюЯяЫы]{3,}/);
  });

  it("stretchProbability=1 always stretches injected words", async () => {
    // With prob=1 and stretchProb=1, every injection is stretched.
    // Stretched text must contain at least one run of 2+ identical Russian vowels.
    const inner = { translate: vi.fn().mockResolvedValue("Один, два, три.") };
    const t = new MatTranslator(inner, 1, 1);
    const result = await t.translate("x");
    expect(result).toMatch(/[АаОоУуЕеЁёИиЭэЮюЯяЫы]{2,}/);
  });

  it("propagates inner translator errors", async () => {
    const inner = { translate: vi.fn().mockRejectedValue(new Error("timeout")) };
    await expect(new MatTranslator(inner, 0.5).translate("x")).rejects.toThrow("timeout");
  });
});

// ─── stretchExpression ──────────────────────────────────────────────────────

describe("stretchExpression", () => {
  it("no Cyrillic vowels → returns expression unchanged", () => {
    expect(stretchExpression("xyz", 3)).toBe("xyz");
  });

  it("single-vowel word: vowel is repeated exactly `repeats` times", () => {
    // "хуй": х(0) у(1) й(2) — only vowel is "у"
    // result: "х" + "у"×3 + "й"
    expect(stretchExpression("хуй", 3)).toMatch(/^х[у]{3}й$/u);
  });

  it("result length grows by (repeats - 1) chars", () => {
    const input = "блядь"; // one vowel "я"
    const result = stretchExpression(input, 4);
    expect(result.length).toBe(input.length + 3); // 4 repeats replaces 1 char with 4
  });

  it("picks most resonant vowel: А beats И", () => {
    // "каит": vowels are "а" (DV index 1) and "и" (DV index 11)
    // topN = ceil(2/2) = 1 → always picks "а" (highest resonance)
    const result = stretchExpression("каит", 3);
    expect(result).toMatch(/^к[а]{3}ит$/u);
  });

  it("output contains a run of 2+ identical Cyrillic vowels", () => {
    const result = stretchExpression("сука", 2);
    expect(result).toMatch(/[АаОоУуЕеЁёИиЭэЮюЯяЫы]{2,}/u);
  });
});

// ─── InterjectTranslator ─────────────────────────────────────────────────────

describe("InterjectTranslator", () => {
  it("prob=0 is a pure passthrough", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Привет мир.") };
    const t = new InterjectTranslator(inner, 0);
    expect(await t.translate("Hello world")).toBe("Привет мир.");
  });

  it("always delegates to inner first", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Привет.") };
    await new InterjectTranslator(inner, 0).translate("Hello");
    expect(inner.translate).toHaveBeenCalledWith("Hello");
  });

  it("prob=1 injects Russian interjections", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Один, два, три.") };
    const result = await new InterjectTranslator(inner, 1).translate("x");
    const words = ["Ого","Ах","Ух","Ой","Эй","Ба","Ишь","О-го-го","эх","уф","ай-яй-яй","вот как","ой","ух","ну","ай"];
    expect(words.some(w => result.includes(w))).toBe(true);
  });

  it("probability defaults to 0.25", () => {
    const t = new InterjectTranslator({ translate: vi.fn() });
    expect(t.probability).toBe(0.25);
  });

  it("probability is public and mutable", () => {
    const t = new InterjectTranslator({ translate: vi.fn() }, 0.1);
    t.probability = 0.9;
    expect(t.probability).toBe(0.9);
  });

  it("propagates inner translator errors", async () => {
    const inner = { translate: vi.fn().mockRejectedValue(new Error("network")) };
    await expect(new InterjectTranslator(inner, 0.5).translate("x")).rejects.toThrow("network");
  });
});

// ─── Pipeline E2E showcase ────────────────────────────────────────────────────
//
// These tests demonstrate the full translator chain as it runs in production:
//   inner → MatTranslator → InterjectTranslator
// They are intentionally prob=1 to make assertions deterministic.

describe("Pipeline E2E showcase: Mat + Interject stacked", () => {
  it("prob=0 on both layers: inner result passes through untouched", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Я люблю этот город.") };
    const result = await new InterjectTranslator(
      new MatTranslator(inner, 0, 0),
      0,
    ).translate("I love this city.");
    expect(result).toBe("Я люблю этот город.");
  });

  it("inner is called exactly once, with the original input text", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Один.") };
    const chain = new InterjectTranslator(new MatTranslator(inner, 0), 0);
    await chain.translate("One.");
    expect(inner.translate).toHaveBeenCalledOnce();
    expect(inner.translate).toHaveBeenCalledWith("One.");
  });

  it("mat+interject at prob=1: output contains both mat and interjection vocabulary", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Один, два, три.") };
    const chain = new InterjectTranslator(
      new MatTranslator(inner, 1, 0), // stretchProb=0 keeps words recognisable
      1,
    );
    const result = await chain.translate("x");

    const matWords    = ["блядь","сука","хуй","пиздец","ёпта","блин","ёб твою мать","нихуя себе","ёбаный в рот","какого хуя","мать твою","ни хуя себе"];
    const interjWords = ["Ого","Ах","Ух","Ой","Эй","Ба","эх","уф","ай-яй-яй","вот как","ой","ух","ну","ай"];

    expect(matWords.some(w    => result.includes(w))).toBe(true);
    expect(interjWords.some(w => result.includes(w))).toBe(true);
  });

  it("stretch at prob=1: injected mat words contain repeated Cyrillic vowels", async () => {
    const inner = { translate: vi.fn().mockResolvedValue("Один, два, три.") };
    const chain = new InterjectTranslator(
      new MatTranslator(inner, 1, 1), // stretchProb=1 — every injection stretched
      0,
    );
    const result = await chain.translate("x");
    expect(result).toMatch(/[АаОоУуЕеЁёИиЭэЮюЯяЫы]{2,}/u);
  });
});

// ─── Panel rendering — snapshot tests ────────────────────────────────────────
//
// Render is stripped of ANSI codes before snapshotting so:
//   • humans can read the snapshot file
//   • ANSI refactors don't create false failures
//   • intentional layout changes get reviewed via `vitest --update-snapshots`

describe("FoniPanel render snapshots", () => {
  it("initial state (TTS off, RVC off, EN)", async () => {
    const { screen } = await mountPanel();
    expect(screen()).toMatchSnapshot();
  });

  it("TTS enabled", async () => {
    const { screen } = await mountPanel(makeState({ enabled: true }));
    expect(screen()).toMatchSnapshot();
  });

  it("RVC enabled with bandit model", async () => {
    const { screen } = await mountPanel(makeState({ rvcEnabled: true, rvcModel: "bandit" }));
    expect(screen()).toMatchSnapshot();
  });

  it("language RU", async () => {
    const { screen } = await mountPanel(makeState({ lang: "ru" }));
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

// ─── Panel behaviour — action call tests ─────────────────────────────────────
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
    const { component, actions } = await mountPanel(makeState({ lang: "en" }));
    await component.handleInput("l");
    expect(actions.setLang).toHaveBeenCalledWith("ru");
  });

  it("l toggles lang ru→en", async () => {
    const { component, actions } = await mountPanel(makeState({ lang: "ru" }));
    await component.handleInput("l");
    expect(actions.setLang).toHaveBeenCalledWith("en");
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

  it("b cycles backend auto→silero", async () => {
    const { component, actions } = await mountPanel(makeState({ backendPref: "auto" }));
    await component.handleInput("b");
    expect(actions.setBackendPref).toHaveBeenCalledWith("silero");
  });

  it("↓ then space activates the language row", async () => {
    const { component, actions } = await mountPanel(makeState({ lang: "en" }));
    await component.handleInput("\x1b[B"); // down → lang row
    await component.handleInput(" ");      // space → activate
    expect(actions.setLang).toHaveBeenCalledWith("ru");
  });
});

// ─── Stale snapshot (known bug) ───────────────────────────────────────────────
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
