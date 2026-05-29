/**
 * Tuning round 6 — pitch naturalness & body.
 *
 * Baseline: round 5 winner (all-derobot) baked into DEFAULT_SMOOTHING.
 * Remaining roboticness sources:
 *   1. Mechanical pitch — espeak F0 is stepped/discrete → vibrato micro-variation
 *   2. Missing body/chest resonance — warmth EQ at 180Hz
 *   3. Missing air — high shelf at 10kHz
 *   4. Dynamics too flat — reduce compression 2:1 → 1.5:1
 *   5. Stack all together
 *
 *   npm run listen 1   # baseline-r5 (DEFAULT_SMOOTHING — all-derobot)
 *   npm run listen 2   # vibrato-subtle: 6Hz / depth=0.003
 *   npm run listen 3   # vibrato-medium: 5Hz / depth=0.006
 *   npm run listen 4   # warmth: +2.5dB@180Hz low shelf
 *   npm run listen 5   # air: +1.5dB@10kHz high shelf
 *   npm run listen 6   # compress-light: ratio 2→1.5
 *   npm run listen 7   # all-r6: vibrato + warmth + air + lighter compression
 */

import { describe, it, beforeAll } from "vitest";
import { EspeakBackend }      from "./backends/espeak.ts";
import { RVCProcessor, SmoothingProcessor, DEFAULT_SMOOTHING, describeSmoothingDiff } from "./pipeline/processors.ts";
import type { SmoothingOptions }    from "./pipeline/processors.ts";
import { SystemPlayer }       from "./pipeline/player.ts";
import { SpeakFacade }        from "./pipeline/speak-facade.ts";
import { IdentityTranslator } from "./pipeline/translators.ts";

const RVC_URL = process.env.RVC_URL ?? "http://127.0.0.1:5050";
const PLAY    = process.env.FONI_PLAY === "1";

const PHRASE = "Ну-ка, чики-брики и в дамке! Понял, брателло?";

// slug = human intent label. name and description are fully auto-generated.
// name  = “${index+1}. ${slug}”
// label = describeSmoothingDiff(opts)
const SLUGS: Array<{ slug: string; opts: Partial<SmoothingOptions> }> = [
  { slug: "baseline-r5",     opts: {} },
  { slug: "vibrato-subtle",  opts: { vibratoFreq: 6, vibratoDepth: 0.003 } },
  { slug: "vibrato-medium",  opts: { vibratoFreq: 5, vibratoDepth: 0.006 } },
  { slug: "warmth",          opts: { warmthBoostDb: 2.5, warmthFreq: 180 } },
  { slug: "air",             opts: { airBoostDb: 1.5, airFreq: 10000 } },
  { slug: "compress-light",  opts: { compressionRatio: 1.5 } },
  { slug: "all-r6",          opts: {
    vibratoFreq: 6, vibratoDepth: 0.003,
    warmthBoostDb: 2, warmthFreq: 180,
    airBoostDb: 1, airFreq: 10000,
    compressionRatio: 1.5,
  }},
];

export const CONFIGS = SLUGS.map(({ slug, opts }, i) => ({
  name:  `${i + 1}. ${slug}`,
  label: describeSmoothingDiff(opts),
  opts,
}));

// ─── Helpers ─────────────────────────────────────────────────────────────────

class NullPlayer {
  detected() { return "null" as const; }
  async play(_buf: Buffer): Promise<void> {}
}

async function isRvcReachable(): Promise<boolean> {
  try {
    const r = await fetch(`${RVC_URL}/params`, { signal: AbortSignal.timeout(2000) });
    return r.ok;
  } catch { return false; }
}

function buildFacade(opts: Partial<SmoothingOptions>): SpeakFacade {
  return new SpeakFacade(
    new IdentityTranslator(),
    new EspeakBackend("ru"),
    new SmoothingProcessor(new RVCProcessor(RVC_URL), opts),
    PLAY ? new SystemPlayer() : new NullPlayer() as unknown as SystemPlayer,
    { voice: "ru", speed: 1.15 },
  );
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("Tuning iterations — rate each 1–5", () => {
  let skip = false;

  beforeAll(async () => {
    const espeakOk = await new EspeakBackend("ru").isAvailable();
    const rvcOk    = await isRvcReachable();
    skip = !espeakOk || !rvcOk;
    if (skip) console.warn("[tuning] espeak or RVC not available — skipping");
    if (PLAY) console.info(`\n[tuning] Round 4: how far can we push?\n[tuning] Phrase: "${PHRASE}"\n`);
  });

  for (const { name, label, opts } of CONFIGS) {
    it(name, async () => {
      if (skip) return;
      if (PLAY) {
        console.info(`\n${"─".repeat(60)}`);
        console.info(`▶  ${name}`);
        console.info(`   ${label}`);
        console.info(`${"─".repeat(60)}`);
      }
      const facade = buildFacade(opts);
      await facade.speak(PHRASE, msg => {
        if (PLAY) console.info(`   ${msg}`);
      });
    }, 120_000);
  }
});
