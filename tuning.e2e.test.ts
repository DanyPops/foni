/**
 * Tuning round 5 — de-robotisation.
 *
 * Research finding: RVC espeak output sounds robotic because:
 *   1. Missing jitter/shimmer (too perfect) → breathiness noise injection
 *   2. Wrong spectral tilt (too flat, too bright) → tilt EQ
 *   3. Metallic sibilant artifacts (S/SH) → de-esser at 7kHz
 *   4. Missing presence (2.5kHz) → presence EQ
 *   5. Exciter at 5kHz adds harshness → move to 1.2kHz for warmth
 *
 * Baseline: round 3 winner (v3 defaults) unchanged.
 * Each variant isolates ONE de-robotisation lever, then all-derobot stacks them.
 *
 *   npm run listen 1   # baseline (round 3 winner — v3 defaults)
 *   npm run listen 2   # breathiness: −45dB noise floor
 *   npm run listen 3   # tilt: +2dB@100Hz / −2dB@8kHz
 *   npm run listen 4   # de-ess: −4dB@7kHz sibilant cut
 *   npm run listen 5   # presence: +1.5dB@2.5kHz
 *   npm run listen 6   # exciter-warm: move 5kHz→1.2kHz
 *   npm run listen 7   # all-derobot: full stack + shorter reverb/phaser
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
  { slug: "baseline-r3",  opts: {} },
  { slug: "breathiness",  opts: { breathinessDb: -45 } },
  { slug: "tilt",         opts: { tiltLowDb: 2, tiltHighDb: -2 } },
  { slug: "deess",        opts: { deEssDb: 4 } },
  { slug: "presence",     opts: { presenceDb: 1.5 } },
  { slug: "exciter-warm", opts: { saturationFreq: 1200 } },
  { slug: "all-derobot",  opts: {
    breathinessDb: -45,
    tiltLowDb: 2, tiltHighDb: -2,
    deEssDb: 4,
    presenceDb: 1.5,
    saturationFreq: 1200,
    reverbMs: 8, reverbDecay: 0.04,
    phaserDepth: 0.08,
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
