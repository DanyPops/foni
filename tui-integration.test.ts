/**
 * Integration tests: TUI frontend ↔ Foni backend.
 *
 * Three scopes, one file — all run without a live server:
 *
 *   WS protocol       — what the extension sends to foni-synth under each trigger
 *   Extension → UI    — inbound WS messages drive setStatus / setWidget
 *   Panel keyboard    — key sequences mutate config and update the WS stream
 *   Re-render wiring  — tui.requestRender() called after every state change
 *
 * Seam boundaries:
 *   ws module          → MockWebSocket (EventEmitter-free, plain listener map)
 *   @earendil-works/pi-tui → same thin mock as unit tests
 *   ExtensionAPI       → FakePi (captures registered handlers + commands)
 *   ExtensionContext   → FakeCtx (records setStatus / setWidget / notify / custom)
 *   global fetch       → vi.stubGlobal, resolved immediately
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// ── Hoisted WS state — shared between mock factory and test body ───────────────

const mockWs = vi.hoisted(() => ({
  instance: null as {
    _fire(event: string, ...args: unknown[]): void;
    readyState: number;
    sent: string[];
  } | null,
  sent: [] as string[],
}));

// ── ws module mock ─────────────────────────────────────────────────────────────

vi.mock("ws", () => {
  class MockWebSocket {
    static OPEN   = 1;
    static CLOSED = 3;

    readyState = MockWebSocket.OPEN;
    readonly sent: string[] = [];
    private readonly _listeners = new Map<string, Function[]>();

    constructor(public readonly url: string) {
      mockWs.instance = this;
      mockWs.sent     = this.sent;
      // Fire "open" after callers have attached listeners.
      process.nextTick(() => this._fire("open"));
    }

    on(event: string, fn: Function): this {
      this._listeners.set(event, [...(this._listeners.get(event) ?? []), fn]);
      return this;
    }

    _fire(event: string, ...args: unknown[]): void {
      for (const fn of this._listeners.get(event) ?? []) fn(...args);
    }

    send(data: string): void { this.sent.push(data); }

    close(): void {
      this.readyState = MockWebSocket.CLOSED;
      this._fire("close");
    }
  }

  return { WebSocket: MockWebSocket };
});

// ── pi-tui mock — identical to unit test mock ──────────────────────────────────

vi.mock("@earendil-works/pi-tui", () => ({
  matchesKey: (data: string, key: unknown): boolean => {
    const MAP: Record<string, string> = {
      up: "\x1b[A", down: "\x1b[B", escape: "\x1b", enter: "\r",
    };
    const k = typeof key === "string" ? key : JSON.stringify(key);
    return data === (MAP[k] ?? k);
  },
  Key: { up: "up", down: "down", escape: "escape", enter: "enter" },
  visibleWidth: (s: string): number =>
    s.replace(/\x1b\[[^m]*m/g, "").length,
}));

// ── Extension under test ───────────────────────────────────────────────────────

import extensionInit from "./index.ts";

// ── Helpers ────────────────────────────────────────────────────────────────────

function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[^m]*m/g, "");
}

/** Yield to the event loop (lets process.nextTick callbacks fire). */
function tick(): Promise<void> {
  return new Promise(r => process.nextTick(r));
}

const KEY = {
  up:     "\x1b[A",
  down:   "\x1b[B",
  escape: "\x1b",
  enter:  "\r",
  space:  " ",
} as const;

// ── Test doubles ───────────────────────────────────────────────────────────────

interface Component {
  render(width: number): string[];
  handleInput(data: string): void;
  invalidate(): void;
}

function makePi() {
  const handlers = new Map<string, Function>();
  const commands = new Map<string, {
    handler: (args: string, ctx: unknown) => Promise<void>;
  }>();

  return {
    on(event: string, fn: Function): void { handlers.set(event, fn); },
    registerCommand(
      name: string,
      def: { handler: (args: string, ctx: unknown) => Promise<void> },
    ): void { commands.set(name, def); },
    emit(event: string, ...args: unknown[]): void {
      handlers.get(event)?.(...args);
    },
    command(name: string) { return commands.get(name)!; },
  };
}

function makeCtx() {
  const requestRender = vi.fn();
  const tui           = { requestRender };
  let   comp: Component | null      = null;
  let   done: ReturnType<typeof vi.fn> = vi.fn();

  const ctx = {
    hasUI: true,
    ui: {
      setStatus: vi.fn(),
      setWidget: vi.fn(),
      notify:    vi.fn(),
      theme:     { fg: (_c: string, t: string) => t },
      custom: vi.fn((factory: Function, _opts: unknown) => {
        done = vi.fn();
        comp = factory(
          tui,
          { fg: (_c: string, t: string) => t },
          {},
          done,
        ) as Component;
      }),
    },
  };

  return {
    ctx,
    tui,
    getComp: () => comp!,
    getDone: () => done,
  };
}

// ── FoniDriver — simulation harness ───────────────────────────────────────────

class FoniDriver {
  private readonly pi:      ReturnType<typeof makePi>;
  private readonly ctxMock: ReturnType<typeof makeCtx>;
  readonly tui: { requestRender: ReturnType<typeof vi.fn> };

  constructor() {
    this.pi      = makePi();
    this.ctxMock = makeCtx();
    this.tui     = this.ctxMock.tui;
  }

  get ctx()  { return this.ctxMock.ctx; }
  get comp() { return this.ctxMock.getComp(); }
  get done() { return this.ctxMock.getDone(); }

  async init(): Promise<void> {
    await extensionInit(this.pi as never);
  }

  /** Fire session_start and wait for the WS to open. */
  async boot(): Promise<void> {
    this.pi.emit("session_start", {}, this.ctx);
    await tick(); // MockWebSocket fires "open" on process.nextTick
  }

  /** Open the /tts panel (no sub-command). */
  async openPanel(): Promise<void> {
    await this.pi.command("tts").handler("", this.ctx);
  }

  /** Run a /tts sub-command (e.g. "enable", "lang en ru"). */
  async runCmd(args: string): Promise<void> {
    await this.pi.command("tts").handler(args, this.ctx);
  }

  /** Emit a pi lifecycle event. */
  emit(event: string, payload: object): void {
    this.pi.emit(event, payload, this.ctx);
  }

  /** Send a key to the open panel and flush async. */
  async press(key: string): Promise<void> {
    this.comp?.handleInput(key);
    await tick();
    await tick(); // second tick for async actions (test ping, etc.)
  }

  /** Navigate from cursor=0 to target item index. */
  async navTo(index: number): Promise<void> {
    for (let i = 0; i < index; i++) await this.press(KEY.down);
  }

  /** Render the panel and strip ANSI codes. */
  render(): string {
    return this.comp?.render(54).map(stripAnsi).join("\n") ?? "";
  }

  /** All WS messages the extension has sent, parsed. */
  wsSent(): Array<Record<string, unknown>> {
    return mockWs.sent.map(s => JSON.parse(s) as Record<string, unknown>);
  }

  /** Reset the sent-message buffer (clears in-place to preserve the shared reference). */
  clearSent(): void { mockWs.sent.length = 0; }

  /** Simulate an inbound WS message from foni-synth. */
  wsInject(msg: object): void {
    mockWs.instance?._fire("message", Buffer.from(JSON.stringify(msg)));
  }

  /** Simulate WS disconnect. */
  wsDisconnect(): void { mockWs.instance?.close(); }
}

// ── Suite setup ───────────────────────────────────────────────────────────────

let driver: FoniDriver;

beforeEach(async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  vi.clearAllMocks();
  mockWs.instance = null;
  mockWs.sent     = [];
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: vi.fn().mockResolvedValue({ models: [] }),
    }),
  );
  driver = new FoniDriver();
  await driver.init();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// ══════════════════════════════════════════════════════════════════════════════
// Scope 1 — WS protocol: what the extension sends to foni-synth
// ══════════════════════════════════════════════════════════════════════════════

describe("WS protocol — outbound messages", () => {
  it("boot() connects WebSocket", async () => {
    await driver.boot();
    expect(mockWs.instance).not.toBeNull();
  });

  it("assistant text_delta → {type:delta} sent", async () => {
    await driver.boot();
    await driver.runCmd("enable");
    driver.clearSent();
    driver.emit("message_update", {
      message: { role: "assistant" },
      assistantMessageEvent: { type: "text_delta", delta: "Hello" },
    });
    expect(driver.wsSent()).toContainEqual({ type: "delta", text: "Hello" });
  });

  it("message_update ignored when TTS disabled", async () => {
    await driver.boot();
    await driver.runCmd("disable");
    driver.clearSent();
    driver.emit("message_update", {
      message: { role: "assistant" },
      assistantMessageEvent: { type: "text_delta", delta: "Hello" },
    });
    expect(driver.wsSent().filter(m => m["type"] === "delta")).toHaveLength(0);
  });

  it("message_update ignored when muted", async () => {
    await driver.boot();
    await driver.runCmd("mute");
    driver.clearSent();
    driver.emit("message_update", {
      message: { role: "assistant" },
      assistantMessageEvent: { type: "text_delta", delta: "Hello" },
    });
    expect(driver.wsSent().filter(m => m["type"] === "delta")).toHaveLength(0);
  });

  it("message_end assistant → {type:message_end} sent", async () => {
    await driver.boot();
    driver.clearSent();
    driver.emit("message_end", { message: { role: "assistant" } });
    expect(driver.wsSent()).toContainEqual({ type: "message_end" });
  });

  it("agent_start → {type:reset} sent", async () => {
    await driver.boot();
    driver.clearSent();
    driver.emit("agent_start", {});
    expect(driver.wsSent()).toContainEqual({ type: "reset" });
  });

  it("/tts enable → {type:set_config, enabled:true} sent", async () => {
    await driver.boot();
    await driver.runCmd("disable"); // ensure it starts off
    driver.clearSent();
    await driver.runCmd("enable");
    expect(driver.wsSent()).toContainEqual({ type: "set_config", enabled: true });
  });

  it("/tts disable → {type:set_config, enabled:false} sent", async () => {
    await driver.boot();
    driver.clearSent();
    await driver.runCmd("disable");
    expect(driver.wsSent()).toContainEqual({ type: "set_config", enabled: false });
  });

  it("/tts stop → {type:reset} sent", async () => {
    await driver.boot();
    driver.clearSent();
    await driver.runCmd("stop");
    expect(driver.wsSent()).toContainEqual({ type: "reset" });
  });

  it("/tts lang en ru → {type:config, key:lang, value:'en,ru'} sent", async () => {
    await driver.boot();
    driver.clearSent();
    await driver.runCmd("lang en ru");
    expect(driver.wsSent()).toContainEqual({ type: "config", key: "lang", value: "en,ru" });
  });

  it("panel space on TTS row → {type:set_config, enabled:false} sent", async () => {
    await driver.boot();
    await driver.runCmd("enable"); // start from ON so toggle goes ON→OFF
    await driver.openPanel();
    driver.clearSent();
    await driver.press(KEY.space); // cursor=0 = TTS/enabled
    expect(driver.wsSent()).toContainEqual({ type: "set_config", enabled: false });
  });

  it("panel space on Stop row → {type:reset} sent", async () => {
    await driver.boot();
    await driver.openPanel();
    await driver.navTo(4); // Stop is item index 4
    driver.clearSent();
    await driver.press(KEY.space);
    expect(driver.wsSent()).toContainEqual({ type: "reset" });
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// Scope 2 — Extension → UI: inbound WS messages drive status + widgets
// ══════════════════════════════════════════════════════════════════════════════

describe("Extension → UI side-effects", () => {
  it("boot → setStatus('tts', …) called", async () => {
    await driver.boot();
    expect(driver.ctx.ui.setStatus).toHaveBeenCalledWith("tts", expect.any(String));
  });

  it("prewarm_start → setStatus('tts-warm', warm message)", async () => {
    await driver.boot();
    vi.clearAllMocks();
    driver.wsInject({ type: "prewarm_start" });
    const calls = (driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls;
    const warmCall = calls.find(([key]: [string]) => key === "tts-warm");
    expect(warmCall).toBeDefined();
    expect(typeof warmCall![1]).toBe("string");
    expect(warmCall![1]).toMatch(/warm/i);
  });

  it("prewarm_done → setStatus('tts-warm', undefined)", async () => {
    await driver.boot();
    vi.clearAllMocks();
    driver.wsInject({ type: "prewarm_done" });
    const calls = (driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls;
    const clearCall = calls.find(([key, val]: [string, unknown]) => key === "tts-warm" && val === undefined);
    expect(clearCall).toBeDefined();
  });

  it("buffer_state active → setWidget('foni-buffer', factory, {placement:'belowEditor'})", async () => {
    await driver.boot();
    driver.wsInject({
      type: "buffer_state",
      data: { slots: [true, false, true], buffered: 2, pending: 1, complete: false },
    });
    expect(driver.ctx.ui.setWidget).toHaveBeenCalledWith(
      "foni-buffer",
      expect.any(Function),
      expect.objectContaining({ placement: "belowEditor" }),
    );
  });

  it("buffer_state complete:true → setWidget('foni-buffer', undefined)", async () => {
    await driver.boot();
    driver.wsInject({
      type: "buffer_state",
      data: { slots: [], buffered: 0, pending: 0, complete: true },
    });
    expect(driver.ctx.ui.setWidget).toHaveBeenCalledWith("foni-buffer", undefined);
  });

  it("buffer_state empty slots complete:false → setWidget('foni-buffer', undefined)", async () => {
    await driver.boot();
    driver.wsInject({
      type: "buffer_state",
      data: { slots: [], buffered: 0, pending: 0, complete: false },
    });
    expect(driver.ctx.ui.setWidget).toHaveBeenCalledWith("foni-buffer", undefined);
  });

  it("/tts mute → setStatus('tts') contains 🔇", async () => {
    await driver.boot();
    await driver.runCmd("enable");
    vi.clearAllMocks();
    await driver.runCmd("mute");
    const calls = (driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls;
    const ttsCall = [...calls].reverse().find(([key]: [string]) => key === "tts");
    expect(ttsCall?.[1]).toContain("🔇");
  });

  it("/tts unmute → setStatus('tts') does not contain 🔇", async () => {
    await driver.boot();
    await driver.runCmd("mute");
    vi.clearAllMocks();
    await driver.runCmd("unmute");
    const calls = (driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls;
    const ttsCall = [...calls].reverse().find(([key]: [string]) => key === "tts");
    expect(ttsCall?.[1]).not.toContain("🔇");
  });

  it("/tts lang en ru → setStatus('tts') contains EN→RU", async () => {
    await driver.boot();
    await driver.runCmd("enable");
    vi.clearAllMocks();
    await driver.runCmd("lang en ru");
    const calls = (driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls;
    const ttsCall = [...calls].reverse().find(([key]: [string]) => key === "tts");
    expect(ttsCall?.[1]).toContain("EN→RU");
  });

  it("/tts status → notify contains 'enabled' and 'ws'", async () => {
    await driver.boot();
    await driver.runCmd("status");
    const [msg] = (driver.ctx.ui.notify as ReturnType<typeof vi.fn>).mock.calls[0] as [string, string];
    expect(msg).toContain("enabled");
    expect(msg).toContain("ws");
  });

  it("panel toggle mat off → next setStatus('tts') excludes +mat", async () => {
    await driver.boot();
    await driver.openPanel();
    await driver.navTo(2); // Mat = item index 2
    vi.clearAllMocks();
    await driver.press(KEY.space); // matEnabled was true → now false
    const calls = (driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls;
    const ttsCall = [...calls].reverse().find(([key]: [string]) => key === "tts");
    expect(ttsCall?.[1]).not.toContain("+mat");
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// Scope 3 — Panel keyboard → backend: key sequences drive config + render
// ══════════════════════════════════════════════════════════════════════════════

describe("Panel keyboard → backend", () => {
  beforeEach(async () => {
    await driver.boot();
    await driver.runCmd("enable"); // start from ON so panel toggle tests are deterministic
    await driver.openPanel();
  });

  it("panel renders border and title", () => {
    const output = driver.render();
    expect(output).toContain("Foni");
    expect(output).toContain("╭");
    expect(output).toContain("╯");
  });

  it("TTS shows ON initially (enabled via beforeEach)", () => {
    expect(driver.render()).toContain("ON");
  });

  it("↓ changes render (cursor moves)", async () => {
    const before = driver.render();
    await driver.press(KEY.down);
    expect(driver.render()).not.toEqual(before);
  });

  it("↑ from row 0 wraps cursor to last item", async () => {
    const before = driver.render();
    await driver.press(KEY.up);
    expect(driver.render()).not.toEqual(before);
  });

  it("vim j moves cursor down", async () => {
    const before = driver.render();
    await driver.press("j");
    expect(driver.render()).not.toEqual(before);
  });

  it("vim k moves cursor up from row > 0", async () => {
    await driver.press(KEY.down); // go to row 1
    const at1 = driver.render();
    await driver.press("k");     // back to row 0
    expect(driver.render()).not.toEqual(at1);
  });

  it("q closes panel — done() called", async () => {
    const doneRef = driver.done;
    await driver.press("q");
    await tick();
    expect(doneRef).toHaveBeenCalled();
  });

  it("esc closes panel — done() called", async () => {
    const doneRef = driver.done;
    await driver.press(KEY.escape);
    await tick();
    expect(doneRef).toHaveBeenCalled();
  });

  it("space on TTS (index 0) → set_config sent + setStatus updated", async () => {
    // cursor is already at index 0
    driver.clearSent();
    const statusCallsBefore = (driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls.length;
    await driver.press(KEY.space);
    expect(driver.wsSent()).toContainEqual({ type: "set_config", enabled: false });
    expect((driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls.length)
      .toBeGreaterThan(statusCallsBefore);
  });

  it("space on Mute (index 1) → setStatus updated, no WS message sent", async () => {
    await driver.navTo(1);
    driver.clearSent();
    const statusCallsBefore = (driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls.length;
    await driver.press(KEY.space);
    expect((driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls.length)
      .toBeGreaterThan(statusCallsBefore);
    expect(driver.wsSent()).toHaveLength(0);
  });

  it("space on Mat (index 2) → setStatus updated, no WS message sent", async () => {
    await driver.navTo(2);
    driver.clearSent();
    await driver.press(KEY.space);
    expect((driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls.length).toBeGreaterThan(0);
    expect(driver.wsSent()).toHaveLength(0);
  });

  it("space on Interject (index 3) → setStatus updated, no WS message sent", async () => {
    await driver.navTo(3);
    driver.clearSent();
    await driver.press(KEY.space);
    expect((driver.ctx.ui.setStatus as ReturnType<typeof vi.fn>).mock.calls.length).toBeGreaterThan(0);
    expect(driver.wsSent()).toHaveLength(0);
  });

  it("space on Stop (index 4) → {type:reset} sent", async () => {
    await driver.navTo(4);
    driver.clearSent();
    await driver.press(KEY.space);
    expect(driver.wsSent()).toContainEqual({ type: "reset" });
  });

  it("toggle TTS: render flips ON → OFF after space", async () => {
    const ttsLine = (r: string) => r.split("\n").find(l => l.includes("TTS"))!;
    expect(ttsLine(driver.render())).toContain("ON");
    await driver.press(KEY.space);
    expect(ttsLine(driver.render())).toContain("OFF");
    expect(ttsLine(driver.render())).not.toContain("ON");
  });

  it("double toggle TTS: render returns to original state", async () => {
    const original = driver.render();
    await driver.press(KEY.space);
    await driver.press(KEY.space);
    expect(driver.render()).toEqual(original);
  });

  it("enter on Test row (index 5) → fetch /params called + notify", async () => {
    await driver.navTo(5);
    await driver.press(KEY.enter);
    expect(vi.mocked(fetch)).toHaveBeenCalledWith(
      expect.stringContaining("/params"),
      expect.any(Object),
    );
    expect(driver.ctx.ui.notify).toHaveBeenCalled();
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// Scope 4 — Re-render notification: tui.requestRender() called after changes
// ══════════════════════════════════════════════════════════════════════════════

describe("Re-render notification", () => {
  beforeEach(async () => {
    await driver.boot();
    await driver.openPanel();
  });

  it("↓ navigation → requestRender called", async () => {
    await driver.press(KEY.down);
    expect(driver.tui.requestRender).toHaveBeenCalled();
  });

  it("↑ navigation → requestRender called", async () => {
    await driver.press(KEY.up);
    expect(driver.tui.requestRender).toHaveBeenCalled();
  });

  it("j navigation → requestRender called", async () => {
    await driver.press("j");
    expect(driver.tui.requestRender).toHaveBeenCalled();
  });

  it("k navigation → requestRender called", async () => {
    await driver.press(KEY.down); // move to row 1 first
    driver.tui.requestRender.mockClear();
    await driver.press("k");
    expect(driver.tui.requestRender).toHaveBeenCalled();
  });

  it("space toggle → requestRender called", async () => {
    await driver.press(KEY.space);
    expect(driver.tui.requestRender).toHaveBeenCalled();
  });

  it("comp.invalidate() → requestRender called", () => {
    driver.comp.invalidate();
    expect(driver.tui.requestRender).toHaveBeenCalled();
  });
});
