/**
 * Tuning — noise floor variants, A/B vs Sidorovich original.
 *
 * breathinessDb at -43 dB causes audible white noise and fails the
 * no-static-noise RED criterion (noiseFloorRatio > 0.02).
 *
 * Each variant plays: [original game WAV] → [our synthesis]
 *
 *   npm run listen 1   # noise-a:   -48 dB — subtle air
 *   npm run listen 2   # noise-b:   -50 dB — barely-there
 *   npm run listen 3   # noise-off:   0    — silent between words
 */

import { describe, it, beforeAll }    from "vitest";
import { readFileSync }                from "node:fs";
import { EspeakBackend }               from "./backends/espeak.ts";
import { RVCProcessor, SmoothingProcessor, describeSmoothingDiff } from "./pipeline/processors.ts";
import type { SmoothingOptions }       from "./pipeline/processors.ts";
import { SystemPlayer }                from "./pipeline/player.ts";
import type { Player }                 from "./pipeline/interfaces.ts";
import { SpeakFacade }                 from "./pipeline/speak-facade.ts";
import { IdentityTranslator }          from "./pipeline/translators.ts";

const RVC_URL = process.env.RVC_URL ?? "http://127.0.0.1:5050";
const PLAY    = process.env.FONI_PLAY === "1";

// Sidorovich trader1a — "Подойди-ка, надо тебе ситуацию прояснить."
// This is the exact WAV used for the acoustic baseline tensor.
const PHRASE      = "Подойди-ка, надо тебе ситуацию прояснить.";
const ORIGINAL_WAV = "baseline/stalker/wav/sidorovich/trader1a.wav";

// slug = human intent label. name and description are fully auto-generated.
// name  = "${index+1}. ${slug}"
// label = describeSmoothingDiff(opts)
const SLUGS: Array<{ slug: string; opts: Partial<SmoothingOptions> }> = [
  { slug: "noise-a",   opts: { breathinessDb: -48 } },
  { slug: "noise-b",   opts: { breathinessDb: -50 } },
  { slug: "noise-off", opts: { breathinessDb: 0   } },
];

export const CONFIGS = SLUGS.map(({ slug, opts }, i) => ({
  name:  `${i + 1}. ${slug}`,
  label: describeSmoothingDiff(opts),
  opts,
}));

// ─── Helpers ─────────────────────────────────────────────────────────────────

/** Silent player used when FONI_PLAY is not set. Implements Player so no cast is needed. */
class NullPlayer implements Player {
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
    PLAY ? new SystemPlayer() : new NullPlayer(),
    { voice: "ru", speed: 1.15 },
  );
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("Tuning iterations - rate each 1-5", () => {
  let skip = false;

  beforeAll(async () => {
    const espeakOk = await new EspeakBackend("ru").isAvailable();
    const rvcOk    = await isRvcReachable();
    skip = !espeakOk || !rvcOk;
    if (skip) console.warn("[tuning] espeak or RVC not available - skipping");
    if (PLAY) console.info(`\n[tuning] Round 4: how far can we push?\n[tuning] Phrase: "${PHRASE}"\n`);
  });

  for (const { name, label, opts } of CONFIGS) {
    it(name, async () => {
      if (skip) return;
      if (PLAY) {
        console.info(`\n${"─".repeat(60)}`);
        console.info(`▶  ${name}  —  ${label}`);
        console.info(`   "${PHRASE}"`);
        console.info(`${"─".repeat(60)}`);

        const player = new SystemPlayer();

        console.info(`   [1/2] original game WAV`);
        await player.play(readFileSync(ORIGINAL_WAV));

        console.info(`   [2/2] our synthesis`);
      }
      const facade = buildFacade(opts);
      await facade.speak(PHRASE, msg => {
        if (PLAY) console.info(`   ${msg}`);
      });
    }, 120_000);
  }
});
