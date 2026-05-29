/**
 * Tuning round 2 — variants on baseline v2 (natural-dry + de-harsh + punch).
 *
 * Baseline: pad + fade + highpass(80Hz) + de-harsh(-2dB@3.5kHz)
 *         + compression(2:1, 20ms attack, +1dB makeup) + loudnorm
 *
 * Each variant tweaks ONE parameter to explore what's still missing.
 *
 *   npm run listen:1   # baseline v2
 *   npm run listen:2   # de-harsh deeper (-3dB)
 *   npm run listen:3   # de-harsh higher freq (4kHz)
 *   npm run listen:4   # air shelf (8kHz +1dB)
 *   npm run listen:5   # tiny reverb (8ms)
 *   npm run listen:6   # de-harsh AND air combined
 *   npm run listen:all # all 6
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

const CONFIGS: Array<{ name: string; label: string; opts: Partial<SmoothingOptions> }> = [
  {
    name:  "1. baseline",
    label: "v2 baseline — natural-dry + de-harsh(-2dB@3.5kHz) + punch(2:1, 20ms)",
    opts:  {},
  },
  {
    name:  "2. harder-cut",
    label: "De-harsh deeper: -3dB instead of -2dB at 3.5kHz",
    opts:  { deHarshDb: -3 },
  },
  {
    name:  "3. higher-cut",
    label: "De-harsh at 4kHz instead of 3.5kHz — target slightly brighter edge",
    opts:  { deHarshFreq: 4000 },
  },
  {
    name:  "4. air",
    label: "Baseline + high shelf +1.5dB at 8kHz — adds sparkle above the cut",
    opts:  { airBoostDb: 1.5, airFreq: 8000 },
  },
  {
    name:  "5. reverb",
    label: "Baseline + tiny room (8ms / 4% decay) — spatial depth",
    opts:  { reverbMs: 8, reverbDecay: 0.04, reverbInputGain: 0.8, reverbOutputGain: 0.88 },
  },
  {
    name:  "6. cut-and-air",
    label: "De-harsh -3dB at 4kHz AND air +1.5dB at 8kHz — cut the bad, add the good",
    opts:  { deHarshFreq: 4000, deHarshDb: -3, airBoostDb: 1.5, airFreq: 8000 },
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
