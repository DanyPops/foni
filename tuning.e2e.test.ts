/**
 * Tuning iterations — SmoothingOptions A/B comparison.
 *
 * Each config targets a different audio theory hypothesis.
 * Run with FONI_PLAY=1 and rate each 1-5.
 *
 *   FONI_PLAY=1 RVC_URL=http://127.0.0.1:5050 npx vitest run tuning.e2e -t "1\."
 *   FONI_PLAY=1 RVC_URL=http://127.0.0.1:5050 npx vitest run tuning.e2e
 */

import { describe, it, beforeAll } from "vitest";
import { EspeakBackend }    from "./backends/espeak.ts";
import { RVCProcessor, SmoothingProcessor, DEFAULT_SMOOTHING } from "./pipeline/processors.ts";
import type { SmoothingOptions }                               from "./pipeline/processors.ts";
import { SystemPlayer }     from "./pipeline/player.ts";
import { SpeakFacade }      from "./pipeline/speak-facade.ts";
import { IdentityTranslator } from "./pipeline/translators.ts";

const RVC_URL = process.env.RVC_URL ?? "http://127.0.0.1:5050";
const PLAY    = process.env.FONI_PLAY === "1";

// ─── Test phrase ─────────────────────────────────────────────────────────────
// Good for hearing: consonants (ч, к, б, т), vowels, phrase rhythm
const PHRASE = "Ну-ка, чики-брики и в дамке! Понял, брателло?";

// ─── Config presets ───────────────────────────────────────────────────────────

const CONFIGS: Array<{ name: string; label: string; opts: Partial<SmoothingOptions> }> = [
  {
    name:  "1. baseline",
    label: "Current defaults — the starting point",
    opts:  {},  // DEFAULT_SMOOTHING as-is
  },
  {
    name:  "2. de-mud",
    label: "Cut 300Hz mud (not 900Hz), kill warmth boost, add 2.5kHz presence — fixes 'potato mouth'",
    opts: {
      warmthBoostDb:         0,      // OFF — warmth boost was creating mud
      deBoxFreq:             300,    // real mud frequency (200-500Hz range)
      deBoxDb:               -4,     // cut harder
      deBoxBandwidthOctaves: 1.0,    // surgical
      deHarshFreq:           3500,
      deHarshDb:             0,      // stop cutting presence — we need it
      eqFreq:                2500,   // boost consonant clarity instead
      eqGain:                2,      // +2dB presence boost
      eqBandwidthOctaves:    2,
      highpassFreq:          100,    // higher cutoff to clear more mud
    },
  },
  {
    name:  "3. consonant-forward",
    label: "No warmth, no deHarsh, strong presence boost, light compression — maximise consonant clarity",
    opts: {
      warmthBoostDb:         0,      // no mud
      airBoostDb:            2.0,    // more air/sparkle
      deBoxFreq:             400,    // cut boxy buildup
      deBoxDb:               -3,
      deHarshDb:             0,      // don't cut presence
      eqFreq:                3000,   // boost intelligibility zone
      eqGain:                3,      // strong presence boost
      compressionRatio:      2,      // lighter — let transients through
      compressionAttackMs:   30,     // slower attack = more consonant punch
      compressionReleaseMs:  100,    // faster release = snappier
      highpassFreq:          120,    // clear the mud floor
      phaserDepth:           0,      // no phaser
      reverbMs:              8,      // shorter reverb
    },
  },
  {
    name:  "4. broadcast",
    label: "Radio announcer EQ — tight low cut, surgical mud cut, strong presence, no exciter",
    opts: {
      highpassFreq:          130,    // broadcast standard low cut
      deBoxFreq:             350,    // broadcast 'proximity effect' cut
      deBoxDb:               -3,
      deBoxBandwidthOctaves: 1.0,
      warmthBoostDb:         0,      // no warmth
      eqFreq:                2000,   // broadcast presence zone
      eqGain:                2.5,
      deHarshFreq:           5000,   // cut sibilance instead of presence
      deHarshDb:             -1.5,
      airBoostDb:            1.5,
      compressionRatio:      2,      // broadcast 2:1
      compressionAttackMs:   10,     // fast — tight control
      compressionReleaseMs:  200,
      saturationDrive:       1.5,    // lighter exciter
      phaserDepth:           0,
      reverbMs:              10,
      reverbDecay:           0.05,
    },
  },
  {
    name:  "5. natural-dry",
    label: "Minimal processing — fade, highpass, light compression, loudnorm. Hear RVC raw.",
    opts: {
      highpassFreq:          80,
      warmthBoostDb:         0,
      airBoostDb:            0,
      deBoxDb:               0,
      deHarshDb:             0,
      eqGain:                0,
      compressionRatio:      1.5,    // barely touching dynamics
      compressionAttackMs:   50,
      saturationDrive:       0,      // no exciter
      saturationAmount:      0,
      phaserDepth:           0,
      reverbMs:              0,      // completely dry
      normalize:             true,
    },
  },
  {
    name:  "6. air-forward",
    label: "Light warmth, big air boost, gentle compression, exciter focused on consonant range",
    opts: {
      highpassFreq:          100,
      deBoxFreq:             300,
      deBoxDb:               -2,
      warmthBoostDb:         1.0,    // subtle warmth only
      warmthFreq:            150,    // deeper, less boxy
      airBoostDb:            2.5,    // strong air
      airFreq:               7000,   // above sibilance
      deHarshDb:             0,
      eqFreq:                2500,   // presence
      eqGain:                1.5,
      compressionRatio:      2,
      compressionAttackMs:   25,
      saturationDrive:       2.0,
      saturationFreq:        4000,   // excite lower — more consonant crunch
      phaserDepth:           0.15,
      reverbMs:              12,
      reverbDecay:           0.06,
    },
  },
];

// ─── Helpers ─────────────────────────────────────────────────────────────────

class NullPlayer {
  detected() { return "null" as const; }
  async play(_: Buffer) {}
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

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("Tuning iterations — rate each 1–5", () => {
  let skip = false;

  beforeAll(async () => {
    const espeakOk = await new EspeakBackend("ru").isAvailable();
    const rvcOk    = await isRvcReachable();
    skip = !espeakOk || !rvcOk;
    if (skip) console.warn("[tuning] espeak or RVC not available — skipping");
    if (PLAY) console.info(`\n[tuning] Phrase: "${PHRASE}"\n`);
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
