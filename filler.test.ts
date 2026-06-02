import { describe, it, expect, beforeEach } from "vitest";
import { FoniEngine }          from "./core/engine.ts";
import { DEFAULT_CONFIG, FILLER_PHRASES } from "./core/config.ts";
import type { FacadePort }     from "./core/interfaces.ts";
import { SpyPlayer, stubBackend, NullProcessor } from "./test/stubs.ts";
import { SpeakFacade }         from "./pipeline/speak-facade.ts";
import { PipelineTranslator }  from "./pipeline/translators.ts";

// ─── Test fixtures ────────────────────────────────────────────────────────────

const RVC_CONFIG = { ...DEFAULT_CONFIG, rvcEnabled: true, outputLang: "ru" as const };
const player = new SpyPlayer();

function makeFillerEngine(): FoniEngine {
  const facadeFactory = async (_cfg: any, translator: any, _emo: any): Promise<FacadePort> =>
    new SpeakFacade(translator, stubBackend, new NullProcessor(), player, {
      voice: "ru", speed: 1.0,
    });
  const translatorFactory = () => new PipelineTranslator([], "ru");
  const processorFactory  = () => new NullProcessor();

  return new FoniEngine(RVC_CONFIG, facadeFactory, translatorFactory, processorFactory);
}

beforeEach(() => player.reset());

// ─── Filler lifecycle ─────────────────────────────────────────────────────────

describe("Filler sounds — pre-warmed RVC fillers during thinking", () => {

  it("FILLER_PHRASES is a non-empty list of Russian strings", () => {
    expect(FILLER_PHRASES.length).toBeGreaterThanOrEqual(5);
    for (const p of FILLER_PHRASES) {
      expect(p.length).toBeGreaterThan(0);
    }
  });

  it("prewarm() populates fillerCache with synthesised buffers", async () => {
    const engine = makeFillerEngine();
    await engine.prewarm();
    expect(engine.fillerCount()).toBeGreaterThan(0);
  });

  it("startFiller() plays one buffer from the cache", async () => {
    const engine = makeFillerEngine();
    await engine.prewarm();
    engine.startFiller();
    // SpyPlayer records played buffers asynchronously — give it a tick
    await new Promise(r => setTimeout(r, 10));
    expect(player.played.length).toBe(1);
  });

  it("stopFiller() calls player.stop()", async () => {
    const engine = makeFillerEngine();
    await engine.prewarm();
    engine.startFiller();
    engine.stopFiller();
    expect(player.stopped).toBe(true);
  });

  it("first onDelta() stops the filler automatically", async () => {
    const engine = makeFillerEngine();
    await engine.prewarm();
    engine.startFiller();
    engine.onDelta("H");
    expect(player.stopped).toBe(true);
  });

  it("startFiller() is a no-op when cache is empty (no prewarm)", () => {
    const engine = makeFillerEngine();
    // No prewarm — fillerCache is empty
    engine.startFiller();
    expect(player.played.length).toBe(0);
  });

  it("reset() stops filler playback", async () => {
    const engine = makeFillerEngine();
    await engine.prewarm();
    engine.startFiller();
    engine.reset();
    expect(player.stopped).toBe(true);
  });

  it("does not play a filler when muted", async () => {
    const engine = makeFillerEngine();
    await engine.prewarm();
    engine.mute();
    engine.startFiller();
    await new Promise(r => setTimeout(r, 10));
    expect(player.played.length).toBe(0);
  });

  it("successive startFiller() calls always play exactly one buffer", async () => {
    const engine = makeFillerEngine();
    await engine.prewarm();
    let count = 0;
    for (let i = 0; i < 10; i++) {
      player.reset();
      engine.startFiller();
      await new Promise(r => setTimeout(r, 5));
      if (player.played.length === 1) count++;
    }
    expect(count).toBe(10);
  });
});
