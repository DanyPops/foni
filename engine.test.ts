import { describe, it, expect } from "vitest";
import { FoniEngine }          from "./core/engine.ts";
import { DEFAULT_CONFIG }      from "./core/config.ts";
import type { BackendFactory } from "./core/engine.ts";
import { stubBackend, NullProcessor, NullPlayer } from "./test/stubs.ts";

const okFactory: BackendFactory  = async () => stubBackend;
const nullFactory: BackendFactory = async () => null;
const okProcessor = () => new NullProcessor();
const stubPlayer  = new NullPlayer();

function makeEngine(factory: BackendFactory = okFactory) {
  return new FoniEngine({ ...DEFAULT_CONFIG }, factory, okProcessor, stubPlayer);
}

// ─── ensureFacade concurrency ─────────────────────────────────────────────────

describe("FoniEngine.ensureFacade", () => {
  it("builds the facade exactly once for sequential calls", async () => {
    let calls = 0;
    const factory: BackendFactory = async () => { calls++; return stubBackend; };
    const engine = makeEngine(factory);

    await engine.ensureFacade();
    await engine.ensureFacade();

    expect(calls).toBe(1);
  });

  it("builds the facade exactly once for concurrent calls", async () => {
    let calls = 0;
    const factory: BackendFactory = async () => {
      calls++;
      await new Promise(r => setTimeout(r, 20)); // expose the race window
      return stubBackend;
    };
    const engine = makeEngine(factory);

    const [f1, f2] = await Promise.all([engine.ensureFacade(), engine.ensureFacade()]);

    expect(calls).toBe(1);
    expect(f1).toBe(f2);
  });

  it("returns the same instance for all concurrent callers", async () => {
    const engine = makeEngine();
    const results = await Promise.all(Array.from({ length: 5 }, () => engine.ensureFacade()));
    const first = results[0];
    expect(results.every(f => f === first)).toBe(true);
  });

  it("invalidateFacade() allows a rebuild", async () => {
    let calls = 0;
    const factory: BackendFactory = async () => { calls++; return stubBackend; };
    const engine = makeEngine(factory);

    await engine.ensureFacade();
    engine.invalidateFacade();
    await engine.ensureFacade();

    expect(calls).toBe(2);
  });

  it("retries when backend is unavailable on first call", async () => {
    let calls = 0;
    const factory: BackendFactory = async () => {
      calls++;
      return calls === 1 ? null : stubBackend;
    };
    const engine = makeEngine(factory);

    const f1 = await engine.ensureFacade();
    const f2 = await engine.ensureFacade();

    expect(f1).toBeNull();
    expect(f2).not.toBeNull();
    expect(calls).toBe(2);
  });

  it("concurrent callers on a null-returning factory all get null without extra builds", async () => {
    let calls = 0;
    const factory: BackendFactory = async () => { calls++; return null; };
    const engine = makeEngine(factory);

    const [f1, f2, f3] = await Promise.all([
      engine.ensureFacade(),
      engine.ensureFacade(),
      engine.ensureFacade(),
    ]);

    expect(calls).toBe(1);
    expect(f1).toBeNull();
    expect(f2).toBeNull();
    expect(f3).toBeNull();
  });
});
