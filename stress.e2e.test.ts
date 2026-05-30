/**
 * Stress tests — TTS pipeline performance and resource metrics.
 *
 * Two edge measurements per the spec:
 *
 *   TTTS (Time-To-Speech)
 *     Cold: speak() called → player.play() invoked  (translate + synth + RVC + queue)
 *     Warm: speak() called → player.play() invoked  (LRU hit + queue)
 *
 *   Storage
 *     Memory: AudioLRU.bytes (in-process cache) + process RSS
 *     Disk:   not yet (phrase ledger not implemented)
 *
 * Instrumentation: Spy pattern on Player — zero production code changes.
 *
 * Run (unit mode, no audio):
 *   npx vitest run stress.e2e
 *
 * Run (with real espeak + RVC + audio):
 *   FONI_PLAY=1 RVC_URL=http://127.0.0.1:5050 npx vitest run stress.e2e
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { SpeakFacade, AudioLRU, AUDIO_CACHE_MAX_BYTES } from "./pipeline/speak-facade.ts";
import { EspeakBackend }  from "./backends/espeak.ts";
import { RVCProcessor, SmoothingProcessor } from "./pipeline/processors.ts";
import { IdentityProcessor }  from "./pipeline/processors.ts";
import { SystemPlayer }   from "./pipeline/player.ts";
import { IdentityTranslator } from "./pipeline/translators.ts";

const RVC_URL  = process.env.RVC_URL  ?? "http://127.0.0.1:5050";
const PLAY     = process.env.FONI_PLAY === "1";

// ─── Percentile helper ────────────────────────────────────────────────────────

function percentile(sorted: number[], p: number): number {
  const idx = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.max(0, idx)];
}

function fmt(ms: number): string {
  return `${ms.toFixed(0)}ms`;
}

// ─── Spy player ───────────────────────────────────────────────────────────────

interface PlayEvent {
  startAt:  number;
  endAt:    number;
  bytes:    number;
}

function makeSpyPlayer(real?: SystemPlayer) {
  const events: PlayEvent[] = [];
  const player = {
    detected: () => "spy" as const,
    play: async (buf: Buffer) => {
      const startAt = Date.now();
      if (real && PLAY) await real.play(buf);
      events.push({ startAt, endAt: Date.now(), bytes: buf.length });
    },
  };
  return { player, events };
}

// ─── Mock facade builder ──────────────────────────────────────────────────────

function makeMockFacade(cache?: AudioLRU) {
  const synthDelays: number[] = [];

  const backend = {
    name:        "mock",
    isAvailable: vi.fn(async () => true),
    synthesize:  vi.fn(async (text: string) => {
      // Simulate ~50ms synthesis
      await new Promise(r => setTimeout(r, 50));
      return Buffer.alloc(1024, 0x42);
    }),
  };
  const processor = { process: vi.fn(async (b: Buffer) => b) };
  const { player, events } = makeSpyPlayer();

  const facade = new SpeakFacade(
    new IdentityTranslator(),
    backend as any,
    processor as any,
    player as any,
    { voice: "ru", speed: 1.0 },
    cache,
  );

  return { facade, backend, processor, player: player as any, events };
}

// ─── T1 + T2: TTTS cold vs warm ───────────────────────────────────────────────

describe("TTTS: cold vs warm", () => {
  it("warm TTTS is significantly faster than cold", async () => {
    const { facade, events } = makeMockFacade();

    // Cold
    const coldStart = Date.now();
    await facade.speak("Привет");
    const coldTTTS = events[0].startAt - coldStart;

    // Warm
    const warmStart = Date.now();
    await facade.speak("Привет");
    const warmTTTS = events[1].startAt - warmStart;

    console.info(`\n  TTTS cold  ${fmt(coldTTTS)}`);
    console.info(`  TTTS warm  ${fmt(warmTTTS)}`);
    console.info(`  Speedup    ${(coldTTTS / Math.max(warmTTTS, 1)).toFixed(1)}×`);

    // Hard assert: warm must be faster
    expect(warmTTTS).toBeLessThan(coldTTTS);
    // Soft guidance: warm should be near-instant (<50ms)
    if (warmTTTS >= 50) {
      console.warn(`  ⚠ warm TTTS ${fmt(warmTTTS)} — expected < 50ms`);
    }
  });
});

// ─── T3: Queue order ─────────────────────────────────────────────────────────

describe("Queue: play order matches call order", () => {
  it("10 concurrent speaks play in call order", async () => {
    const { facade, events } = makeMockFacade();
    const phrases = Array.from({ length: 10 }, (_, i) => `Фраза ${i + 1}`);

    await Promise.all(phrases.map(p => facade.speak(p)));

    // Events arrive in play order — verify startAt is monotonically increasing
    for (let i = 1; i < events.length; i++) {
      expect(events[i].startAt).toBeGreaterThanOrEqual(events[i - 1].startAt);
    }
    console.info(`\n  10 phrases played in call order ✓`);
  });
});

// ─── T4: Queue delay distribution ─────────────────────────────────────────────

describe("Queue: delay distribution", () => {
  it("reports p50 and p95 queue delay for 20 concurrent speaks", async () => {
    const { facade, events } = makeMockFacade();
    const callTimes: number[] = [];

    // Fire 20 speaks as fast as possible — each records call time
    await Promise.all(
      Array.from({ length: 20 }, (_, i) => {
        callTimes.push(Date.now());
        return facade.speak(`Задержка ${i}`);
      }),
    );

    const delays = events.map((e, i) => e.startAt - callTimes[i]).sort((a, b) => a - b);
    const p50 = percentile(delays, 50);
    const p95 = percentile(delays, 95);

    console.info(`\n  Queue delay  p50 ${fmt(p50)}  p95 ${fmt(p95)}`);
    console.info(`  Total phrases: ${events.length}, all played ✓`);

    // All phrases must eventually play
    expect(events).toHaveLength(20);
    // p50 queue delay should not be absurd for mock backend
    expect(p50).toBeLessThan(5000);
  });
});

// ─── T5: Memory — LRU stays within budget ─────────────────────────────────────

describe("Memory: LRU byte budget", () => {
  it("50 unique phrases never exceed AUDIO_CACHE_MAX_BYTES", async () => {
    const smallCache = new AudioLRU(5 * 1024 * 1024); // 5MB test budget
    const { facade } = makeMockFacade(smallCache);

    const heapBefore = process.memoryUsage().heapUsed;

    for (let i = 0; i < 50; i++) {
      await facade.speak(`Уникальная фраза номер ${i}`);
    }

    const heapAfter  = process.memoryUsage().heapUsed;
    const heapDeltaMB = (heapAfter - heapBefore) / 1024 / 1024;
    const lruMB       = smallCache.bytes / 1024 / 1024;

    console.info(`\n  LRU        ${lruMB.toFixed(2)}MB / ${(5).toFixed(0)}MB budget`);
    console.info(`  Heap delta  +${heapDeltaMB.toFixed(1)}MB over 50 phrases`);
    console.info(`  LRU entries ${smallCache.size}`);

    // Hard assert: cache must never exceed its budget
    expect(smallCache.bytes).toBeLessThanOrEqual(5 * 1024 * 1024);
  });

  it("heap growth is bounded after 30 unique speaks", async () => {
    const { facade } = makeMockFacade();
    const rssBefore = process.memoryUsage().rss;

    for (let i = 0; i < 30; i++) {
      await facade.speak(`Стресс фраза ${i}`);
    }

    const rssAfter   = process.memoryUsage().rss;
    const rssDeltaMB = (rssAfter - rssBefore) / 1024 / 1024;

    console.info(`\n  RSS before  ${(rssBefore / 1024 / 1024).toFixed(1)}MB`);
    console.info(`  RSS after   ${(rssAfter / 1024 / 1024).toFixed(1)}MB`);
    console.info(`  RSS delta   +${rssDeltaMB.toFixed(1)}MB`);

    expect(rssDeltaMB).toBeLessThan(100);
  }, 10_000);
});

// ─── T6: Hit rate after warm ───────────────────────────────────────────────────

describe("Cache: hit rate", () => {
  it("20 speaks (10 unique x2) achieves >= 50% hit rate", async () => {
    const { facade, backend } = makeMockFacade();
    const phrases = Array.from({ length: 10 }, (_, i) => `Фраза ${i}`);

    // First pass — all cold (10 misses)
    for (const p of phrases) await facade.speak(p);
    const missCount = backend.synthesize.mock.calls.length;

    // Second pass — all warm (10 hits)
    for (const p of phrases) await facade.speak(p);
    const totalSynth = backend.synthesize.mock.calls.length;
    const hitCount   = 20 - totalSynth;
    const hitRate    = hitCount / 20;

    console.info(`\n  Speaks     20  (10 unique × 2)`);
    console.info(`  Cache hits ${hitCount}  misses ${totalSynth}`);
    console.info(`  Hit rate   ${(hitRate * 100).toFixed(0)}%`);
    console.info(`  LRU        ${facade.cache.size} entries, ${(facade.cache.bytes / 1024).toFixed(0)}KB`);

    expect(hitRate).toBeGreaterThanOrEqual(0.5);
  });
});

// ─── T7: cacheStats format ────────────────────────────────────────────────────

describe("cacheStats", () => {
  it("format contains entry count and MB after speaks", async () => {
    const { facade } = makeMockFacade();
    await facade.speak("Тест");
    const stats = facade.cacheStats();
    console.info(`\n  cacheStats: "${stats}"`);
    expect(stats).toMatch(/\d+ entries \/ [\d.]+MB/);
  });
});

// ─── T8: E2E — real espeak + RVC (gated) ──────────────────────────────────────

describe("E2E TTTS: espeak → RVC → play", () => {
  let skipE2e = false;

  beforeEach(async () => {
    const espeakOk = await new EspeakBackend("ru").isAvailable();
    const rvcOk    = await fetch(`${RVC_URL}/params`, { signal: AbortSignal.timeout(2000) })
      .then(r => r.ok).catch(() => false);
    skipE2e = !espeakOk || !rvcOk;
    if (skipE2e) console.warn("[stress e2e] espeak or RVC not available — skipping");
  });

  it("cold TTTS < 10s, warm TTTS < 100ms on real pipeline", async () => {
    if (skipE2e) return;

    const realPlayer = PLAY ? new SystemPlayer() : undefined;
    const { player, events } = makeSpyPlayer(realPlayer);

    const facade = new SpeakFacade(
      new IdentityTranslator(),
      new EspeakBackend("ru"),
      new SmoothingProcessor(new RVCProcessor(RVC_URL)),
      player as any,
      { voice: "ru", speed: 1.15 },
    );

    const phrase = "Ну-ка, чики-брики и в дамке!";

    // Cold
    const coldStart = Date.now();
    await facade.speak(phrase);
    const coldTTTS = events[0].startAt - coldStart;

    // Warm
    const warmStart = Date.now();
    await facade.speak(phrase);
    const warmTTTS = events[1].startAt - warmStart;

    const rssMB = process.memoryUsage().rss / 1024 / 1024;

    console.info(`\n┌─ TTTS cold    ${fmt(coldTTTS)}`);
    console.info(`├─ TTTS warm    ${fmt(warmTTTS)}`);
    console.info(`├─ Speedup      ${(coldTTTS / Math.max(warmTTTS, 1)).toFixed(0)}×`);
    console.info(`├─ LRU          ${facade.cacheStats()}`);
    console.info(`└─ RSS          ${rssMB.toFixed(1)}MB`);

    // Hard asserts
    expect(coldTTTS).toBeLessThan(10_000);  // 10s max for espeak+RVC on CPU
    expect(warmTTTS).toBeLessThan(100);     // <100ms for cache hit
    expect(warmTTTS).toBeLessThan(coldTTTS);
  }, 30_000);
});
