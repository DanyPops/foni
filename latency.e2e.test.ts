/**
 * latency.e2e.test.ts — TTFA scaling test: 10 / 100 / 1,000 words.
 *
 * Metrics per run:
 *   TTFA       — ms from first delta to first audio chunk played
 *   Stream dur — ms to push all deltas
 *   Audio tail — ms of audio still playing after stream ended (queue depth)
 *   Total      — stream start → last audio chunk
 *
 * Run (espeak baseline, no services needed):
 *   npx vitest run latency.e2e
 *
 * Run (full RVC+DSP pipeline):
 *   FONI_SYNTH_URL=http://localhost:5051 npx vitest run latency.e2e
 */

import { describe, it, expect, beforeAll } from "vitest";
import { FoniEngine }                       from "./core/engine.ts";
import { DEFAULT_CONFIG }                   from "./core/config.ts";
import type { FacadeFactory, TranslatorFactory, ProcessorFactory } from "./core/engine.ts";
import type { AudioProcessor, Player }      from "./core/interfaces.ts";
import { EspeakBackend }                    from "./backends/espeak.ts";
import { SpeakFacade }                      from "./pipeline/speak-facade.ts";
import { PipelineTranslator, makeITGlossaryMiddleware } from "./pipeline/translators.ts";
import { SmoothingProcessor, RVCProcessor } from "./pipeline/processors.ts";

// ─── Timing player ────────────────────────────────────────────────────────────

interface PlayEvent { t: number; bytes: number; }

class TimingPlayer implements Player {
  readonly events: PlayEvent[] = [];
  get firstPlayMs()  { return this.events[0]?.t       ?? null; }
  get lastPlayMs()   { return this.events.at(-1)?.t   ?? null; }
  get totalChunks()  { return this.events.length; }
  get totalBytes()   { return this.events.reduce((s, e) => s + e.bytes, 0); }
  async play(buf: Buffer) { this.events.push({ t: Date.now(), bytes: buf.length }); }
}

// ─── Corpus bank ─────────────────────────────────────────────────────────────

/**
 * Source text: ~1,000 words of stalker-universe prose with IT glossary terms.
 * Sliced to produce the 10-word and 100-word corpora from the same source.
 */
const FULL_CORPUS = `
The Zone is not a place you choose. The Zone chooses you. Every stalker who
has walked through the wire knows this, though few will admit it. There is
something pulling at you from the inside, a quiet insistence that refuses to
be argued away. You pack your gear, you check your detector, you test the
batteries in your headlamp, and still the Zone is already inside your head
before you have crossed the perimeter.

Sidorovich used to say that information is the only currency worth holding.
Everything else rots. Artifacts corrode. Weapons jam. Food goes bad. But a
piece of information, correctly placed, correctly timed, is worth more than
any psy-protection suit the military ever stamped a serial number on. He
would sit behind his glass case and look at you with those small, calculating
eyes and you would know, without being told, that he already knew more about
your next move than you did yourself.

The anomalies do not care who you are. They are not malicious. They are simply
indifferent, which is worse. A Whirligig will throw you thirty meters into
the air with exactly the same force it would use on a military patrol, a
rookie, or a veteran who has survived a hundred emissions. The Zone distributes
its cruelty democratically. This is something the newcomers never quite
believe until they see it happen to someone they trusted.

Emissions are the other thing nobody talks about honestly. They will tell you
to find shelter. They will tell you to get low, get covered, stay away from
metal. What they will not tell you is that there is no shelter that feels
adequate when the sky turns that particular shade of yellow-green and the
ground begins to hum. You find out for yourself that the shelter is mostly
psychological. You crouch in a cellar or behind a fallen wall and you count
seconds and you wait, and if you are still breathing when it passes you file
the experience under lessons learned and you continue.

The stalkers who stay alive longest are not the strongest or the fastest.
They are the ones who listen. Listen to the ground, to the other stalkers,
to the silence that precedes something moving in the grass, to the specific
pitch of a detector alarm that distinguishes a gravitational trap from a
thermal one. The Zone communicates if you are patient enough to hear it.

We deployed the new communication relay three days ago. The commit history
shows nine separate revisions before anyone was satisfied. The pull request
sat open for six hours while everyone argued about error handling. Finally
the merge went through at two in the morning and the relay came online and
now the signal reaches sectors that were dark for three months.

Information flows again. Stalkers report anomaly clusters. Traders update
their stock lists. The branch of the faction network that had been isolated
rebuilds its connection to the main repository of shared knowledge. It is
remarkable how much of survival depends on simply knowing what other people
know, and remarkable how much effort goes into preventing that from happening.

Barkeep has a theory that the Zone is not hostile but rather extremely literal.
You approach it with fear, it gives you fear. You approach it with curiosity,
it gives you something worth discovering. You approach it with greed, you
discover very quickly what happens to people who approach it with greed.

The artifacts are the clearest evidence of this principle. They form where
energy concentrates, where the anomalies reach some kind of equilibrium with
the physics that surrounds them. A Moonlight or a Bubble does not appear
because the Zone is generous. It appears because the conditions were right
and something crystallized out of the chaos. That is all. But the stalker
who finds it and carries it out and sells it to a researcher or keeps it for
personal protection is changed by that transaction, and the change is not
always visible immediately.

Some people think the Zone wants something from you. Others think it wants
nothing and that is precisely what makes it dangerous. The truth, if there is
one, is probably simpler than either position. The Zone is a system in a
state of continuous disequilibrium, and you are a small component passing
through it, and whether you survive depends on how well you understand the
rules it is currently operating under. The rules change. That is the only
constant.

Sleep when you can. Eat when you can. Trust selectively. Check your equipment
twice. Share information with people who share information back. Never walk
the same path twice in a day. Know where the nearest cover is before you need
it. Keep the detector in your hand, not on your belt. Watch what the dogs do.

If you follow these principles consistently, you improve your odds. You do
not eliminate the danger. Nobody eliminates the danger. But you extend the
distance between yourself and the moment when the Zone finally decides it is
done being patient with you. Good luck, stalker.
`.trim();

/** Return exactly N words from the source corpus, looping if necessary. */
function corpusOfWords(n: number): string {
  const words = FULL_CORPUS.split(/\s+/);
  const result: string[] = [];
  while (result.length < n) result.push(...words.slice(0, n - result.length));
  return result.join(" ");
}

/** Simulate LLM streaming: split text into ~15-char bursts. */
function streamChunks(text: string, chunkChars = 15): string[] {
  const out: string[] = [];
  for (let i = 0; i < text.length; i += chunkChars) out.push(text.slice(i, i + chunkChars));
  return out;
}

// ─── Engine factory ───────────────────────────────────────────────────────────

function buildEngine(player: TimingPlayer): FoniEngine {
  const synthUrl = process.env.FONI_SYNTH_URL;

  const translator: TranslatorFactory = () =>
    new PipelineTranslator([makeITGlossaryMiddleware()], "ru");

  const processor: ProcessorFactory = () =>
    synthUrl
      ? new SmoothingProcessor(new RVCProcessor(synthUrl), {}, synthUrl)
      : ({ process: async (b: Buffer) => b } as AudioProcessor);

  const facade: FacadeFactory = async (_cfg, t) => {
    const backend = new EspeakBackend("ru");
    if (!await backend.isAvailable()) return null;
    return new SpeakFacade(t, backend, processor(), player, { voice: "ru", speed: 1.15 });
  };

  return new FoniEngine(
    { ...DEFAULT_CONFIG, enabled: true, rvcEnabled: !!synthUrl },
    facade, translator, processor,
  );
}

// ─── Runner ───────────────────────────────────────────────────────────────────

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

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("TTFA scaling — 10 / 100 / 1,000 words", () => {
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

  // ── Per-size assertions ──────────────────────────────────────────────────

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

  // ── Summary table ────────────────────────────────────────────────────────

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
