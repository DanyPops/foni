/**
 * Tuning iterations — variants on the natural-dry baseline.
 *
 * Baseline (DEFAULT_SMOOTHING): pad + fade + highpass(80Hz) + compression(1.5:1) + loudnorm.
 * Everything else off — RVC carries the voice character.
 *
 * Each variant adds exactly ONE thing on top of the baseline.
 * Goal: find what improves over natural-dry without over-processing.
 *
 *   npm run listen:1   # baseline (natural-dry)
 *   npm run listen:2   # + tiny room reverb
 *   npm run listen:3   # + air (high shelf 8kHz)
 *   npm run listen:4   # + presence (2.5kHz boost)
 *   npm run listen:5   # + de-harsh (3.5kHz cut)
 *   npm run listen:6   # + punchier compression
 *   npm run listen:all # all 6 in sequence
 */

import { describe, it, beforeAll } from "vitest";
import { EspeakBackend }      from "./backends/espeak.ts";
import { RVCProcessor, SmoothingProcessor, DEFAULT_SMOOTHING } from "./pipeline/processors.ts";
import type { SmoothingOptions }    from "./pipeline/processors.ts";
import { SystemPlayer }       from "./pipeline/player.ts";
import { SpeakFacade }        from "./pipeline/speak-facade.ts";
import { IdentityTranslator } from "./pipeline/translators.ts";

const RVC_URL = process.env.RVC_URL ?? "http://127.0.0.1:5050";
const PLAY    = process.env.FONI_PLAY === "1";

const PHRASE = "Ну-ка, чики-брики и в дамке! Понял, брателло?";

// ─── Variants ─────────────────────────────────────────────────────────────────
//
// DEFAULT_SMOOTHING is the natural-dry baseline.
// Each variant overrides exactly one group of fields.

const CONFIGS: Array<{ name: string; label: string; opts: Partial<SmoothingOptions> }> = [
  {
    name:  "1. baseline",
    label: "Natural-dry — pad + fade + highpass + compression 1.5:1 + loudnorm. Everything else off.",
    opts:  {},
  },
  {
    name:  "2. +reverb",
    label: "Baseline + tiny room (8ms / 4% decay) — just enough to feel less 'in a box'",
    opts: {
      reverbMs:         8,
      reverbDecay:      0.04,
      reverbInputGain:  0.8,
      reverbOutputGain: 0.88,
    },
  },
  {
    name:  "3. +air",
    label: "Baseline + high shelf +1.5dB at 8kHz — adds sparkle and breath above RVC",
    opts: {
      airBoostDb: 1.5,
      airFreq:    8000,
    },
  },
  {
    name:  "4. +presence",
    label: "Baseline + peaking +2dB at 2.5kHz — boosts consonant clarity and intelligibility",
    opts: {
      deHarshFreq: 2500,
      deHarshDb:   2,
      deHarshBandwidthOctaves: 2,
    },
  },
  {
    name:  "5. +de-harsh",
    label: "Baseline + peaking -2dB at 3.5kHz — cuts any residual espeak metallic edge",
    opts: {
      deHarshFreq: 3500,
      deHarshDb:   -2,
      deHarshBandwidthOctaves: 2,
    },
  },
  {
    name:  "6. +punch",
    label: "Baseline + faster compression (2:1, 20ms attack) — tighter, punchier dynamics",
    opts: {
      compressionRatio:    2,
      compressionAttackMs: 20,
      compressionMakeupDb: 1,
    },
  },
];

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
