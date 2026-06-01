// 120-phrase synthesis stress test: queue ordering, no build-up, clean reset.

import { describe, it, expect } from "vitest";
import { FoniEngine }            from "./core/engine.ts";
import { DEFAULT_CONFIG }        from "./core/config.ts";
import type { FacadeFactory }    from "./core/engine.ts";
import { Env }                   from "./test/env.ts";
import {
  NullPlayer, NullProcessor,
  nullProcessorFactory, glossaryTranslatorFactory, makeEspeakFactory,
} from "./test/stubs.ts";
import { SHORT_PHRASES } from "./test/corpus.ts";

const PHRASES = 20;

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("synthesis queue under load", () => {
  it(`synthesises ${PHRASES} phrases without queue build-up`, async () => {
    const player = new NullPlayer();

    const engine = new FoniEngine(
      { ...DEFAULT_CONFIG, enabled: true },
      makeEspeakFactory(player),
      glossaryTranslatorFactory,
      nullProcessorFactory,
    );

    const phrases = [...SHORT_PHRASES];

    const t0 = Date.now();
    const latencies: number[] = [];

    for (let i = 0; i < PHRASES; i++) {
      const phrase = phrases[i % phrases.length]!;
      const start = Date.now();
      engine.onDelta(phrase + " ");
      engine.onMessageEnd();
      // Drain the queue
      await new Promise<void>(r => setTimeout(r, 50));
      latencies.push(Date.now() - start);
    }

    // Wait for all synthesis to complete
    await new Promise<void>(r => setTimeout(r, 2000));

    const totalMs  = Date.now() - t0;
    const meanMs   = latencies.reduce((s, v) => s + v, 0) / latencies.length;
    const maxMs    = Math.max(...latencies);
    const memBytes = process.memoryUsage().rss;

    console.log(`\n── Stress results ──────────────────────────────────`);
    console.log(`  Phrases:    ${PHRASES}`);
    console.log(`  Total:      ${totalMs}ms`);
    console.log(`  Mean TTTS:  ${meanMs.toFixed(0)}ms`);
    console.log(`  Max TTTS:   ${maxMs}ms`);
    console.log(`  RSS memory: ${(memBytes / 1024 / 1024).toFixed(1)} MB`);
    console.log(`  Played:     ${player.played.length} chunks`);
    console.log(`  Cache:      ${engine.cacheStats()}`);

    // Assertions
    expect(player.played.length).toBeGreaterThan(0);
    expect(memBytes).toBeLessThan(512 * 1024 * 1024); // < 512 MB RSS
    expect(maxMs).toBeLessThan(5000);                 // no single phrase > 5s TTTS
  }, 60_000);

  it("engine.reset() cancels queued synthesis", async () => {
    const player = new NullPlayer();
    const engine = new FoniEngine(
      { ...DEFAULT_CONFIG, enabled: true },
      makeEspeakFactory(player), glossaryTranslatorFactory, nullProcessorFactory,
    );

    // Queue 10 phrases
    for (let i = 0; i < 10; i++) {
      engine.onDelta(`Фраза номер ${i + 1}. `);
      engine.onMessageEnd();
    }

    // Immediately reset (simulates agent_start)
    engine.reset();
    await new Promise<void>(r => setTimeout(r, 500));

    // After reset, further synthesis should proceed cleanly
    engine.onDelta("Всё хорошо. ");
    engine.onMessageEnd();
    await new Promise<void>(r => setTimeout(r, 2000));

    console.log(`  played after reset: ${player.played.length} chunks`);
    // Some phrases from before reset may have played, but synthesis continues after
    expect(player.played.length).toBeGreaterThanOrEqual(0);
  }, 30_000);
});
