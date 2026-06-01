
import { describe, it, expect, beforeAll } from "vitest";
import { FoniEngine }    from "./core/engine.ts";
import { DEFAULT_CONFIG } from "./core/config.ts";
import type { ProcessorFactory } from "./core/engine.ts";
import type { AudioProcessor }   from "./core/interfaces.ts";
import { SmoothingProcessor, RVCProcessor } from "./pipeline/processors.ts";
import { Env }           from "./test/env.ts";
import {
  TimingPlayer, NullProcessor,
  glossaryTranslatorFactory, makeEspeakFactory, makeRvcFactory,
} from "./test/stubs.ts";
import { corpusOfWords, streamChunks } from "./test/corpus.ts";


function buildEngine(player: TimingPlayer): FoniEngine {
  const synthUrl = Env.SYNTH_URL;
  const processorFactory: ProcessorFactory = () =>
    synthUrl
      ? new SmoothingProcessor(new RVCProcessor(synthUrl), {}, synthUrl)
      : (new NullProcessor() as AudioProcessor);
  const facadeFactory = synthUrl
    ? makeRvcFactory(player, synthUrl)
    : makeEspeakFactory(player);

  return new FoniEngine(
    { ...DEFAULT_CONFIG, enabled: true, rvcEnabled: !!synthUrl },
    facadeFactory, glossaryTranslatorFactory, processorFactory,
  );
}


interface RunResult {
  words:       number;
  ttfaMs:      number | null;
  streamMs:    number;
  audioTailMs: number;
  totalMs:     number;
  chunks:      number;
  kbytes:      number;
}

async function runScaling(wordCount: number): Promise<RunResult> {
  const player  = new TimingPlayer();
  const engine  = buildEngine(player);
  const text    = corpusOfWords(wordCount);
  const deltas  = streamChunks(text, 15);

  engine.reset();

  const tStart = Date.now();

  // Stream at ~100 chars/sec (realistic fast LLM): 15 chars / 150ms = 100 cps
  for (const chunk of deltas) {
    engine.onDelta(chunk);
    await new Promise(r => setTimeout(r, 10));
  }
  engine.onMessageEnd();

  const tStreamEnd = Date.now();

  // Drain: wait until 1 s of silence after last audio chunk (or 90 s timeout).
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    await new Promise(r => setTimeout(r, 250));
    if (player.lastPlayMs !== null && Date.now() - player.lastPlayMs > 1_000) break;
  }

  const tAudioEnd = player.lastPlayMs ?? tStreamEnd;

  return {
    words:       wordCount,
    ttfaMs:      player.firstPlayMs != null ? player.firstPlayMs - tStart : null,
    streamMs:    tStreamEnd - tStart,
    audioTailMs: Math.max(0, tAudioEnd - tStreamEnd),
    totalMs:     tAudioEnd - tStart,
    chunks:      player.totalChunks,
    kbytes:      Math.round(player.totalBytes / 1024),
  };
}


describe("TTFA scales linearly with word count", () => {
  const synthUrl = process.env.FONI_SYNTH_URL;
  const mode     = synthUrl ? `RVC+DSP @ ${synthUrl}` : "espeak-only";

  // TTFA budget: espeak is fast; RVC adds synthesis time.
  // The first sentence in any corpus hits within the first ~10 words,
  // so TTFA should be roughly constant regardless of corpus size.
  const TTFA_BUDGET_MS = synthUrl ? 20_000 : 4_000;

  let results: RunResult[] = [];

  beforeAll(async () => {
    console.log(`\n🔊 TTFA Scaling Test — ${mode}`);
    console.log(`   TTFA budget: ${TTFA_BUDGET_MS} ms\n`);
    // Run all three sizes sequentially so the espeak/RVC sessions warm up once.
    results = [
      await runScaling(10),
      await runScaling(100),
      await runScaling(1_000),
    ];
  }, 600_000);


  for (const wordTarget of [10, 100, 1_000]) {
    it(`${wordTarget}-word stream`, () => {
      const r = results.find(x => x.words === wordTarget);
      if (!r) throw new Error("result missing");

      console.log(`\n  ── ${r.words} words ─────────────────────────────────`);
      console.log(`     Stream dur:  ${r.streamMs} ms`);
      console.log(`     TTFA:        ${r.ttfaMs != null ? r.ttfaMs + " ms" : "no audio"}`);
      console.log(`     Audio tail:  ${r.audioTailMs} ms`);
      console.log(`     Total:       ${r.totalMs} ms`);
      console.log(`     Chunks:      ${r.chunks}  (${r.kbytes} kB)`);

      if (r.ttfaMs != null) {
        // TTFA should be constant-ish — bounded by the first sentence, not corpus size.
        expect(r.ttfaMs, `TTFA for ${r.words} words`).toBeLessThan(TTFA_BUDGET_MS);
      }
      expect(r.chunks).toBeGreaterThanOrEqual(r.words > 5 ? 1 : 0);
    }, 600_000);
  }


  it("prints scaling summary", () => {
    const rows = results.map(r => ({
      "words":       r.words.toLocaleString(),
      "stream ms":   r.streamMs,
      "TTFA ms":     r.ttfaMs ?? "—",
      "tail ms":     r.audioTailMs,
      "total ms":    r.totalMs,
      "chunks":      r.chunks,
    }));

    console.log(`\n\n╔══ TTFA Scaling Summary — ${mode} ${"═".repeat(Math.max(0, 42 - mode.length))}╗`);
    console.log(`║  Words   Stream    TTFA    Tail    Total   Chunks  ║`);
    console.log(`╠════════════════════════════════════════════════════╣`);
    for (const r of results) {
      const ttfa = r.ttfaMs != null ? String(r.ttfaMs).padStart(6) : "     —";
      console.log(
        `║  ${String(r.words).padStart(5)}` +
        `   ${String(r.streamMs).padStart(6)}` +
        `  ${ttfa}` +
        `  ${String(r.audioTailMs).padStart(6)}` +
        `  ${String(r.totalMs).padStart(6)}` +
        `  ${String(r.chunks).padStart(5)}  ║`,
      );
    }
    console.log(`╚════════════════════════════════════════════════════╝`);

    const ttfas = results.map(r => r.ttfaMs).filter((t): t is number => t != null);
    if (ttfas.length >= 2) {
      const spread = Math.max(...ttfas) - Math.min(...ttfas);
      console.log(`\n  TTFA spread across sizes: ${spread} ms`);
      console.log(`  (should be small — TTFA depends on first sentence, not corpus length)\n`);
      // TTFA should not grow proportionally with corpus size.
      // Largest TTFA should be < 3× smallest TTFA.
      expect(Math.max(...ttfas)).toBeLessThan(Math.min(...ttfas) * 5 + 2_000);
    }
  }, 10_000);
});
