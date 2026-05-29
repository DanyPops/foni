/**
 * Tuning round 3 — targeting robotic voice specifically.
 *
 * Baseline: natural-dry + de-harsh(-2dB@3.5kHz) + punch(2:1, 20ms)
 *
 * Root cause of robotic quality:
 *   - espeak formant synthesis: perfectly timed, flat harmonics
 *   - no organic imperfections, no breath, no micro-variation
 *
 * Three levers that specifically fight roboticness:
 *   aexciter  — adds synthetic harmonics above a threshold freq
 *               brain interprets harmonic richness as "warmth/natural"
 *   aphaser   — subtle phase movement breaks flat formant pattern
 *               adds micro-variation that organic voices have naturally
 *   reverb    — room presence — brain interprets dry=microphone=robotic
 *               even very small room makes voice feel more natural
 *
 *   npm run listen:1   # baseline
 *   npm run listen:2   # + light exciter (drive=1.5 @ 5kHz)
 *   npm run listen:3   # + phaser (depth=0.15)
 *   npm run listen:4   # + reverb (12ms / 6% decay)
 *   npm run listen:5   # exciter + phaser combined
 *   npm run listen:6   # all three: exciter + phaser + reverb
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
    label: "v2 baseline — natural-dry + de-harsh(-2dB@3.5kHz) + punch(2:1, 20ms). Reference point.",
    opts:  {},
  },
  {
    name:  "2. exciter",
    label: "Harmonic exciter: drive=1.5 above 5kHz. Adds subtle odd harmonics — brain reads as warmth.",
    opts:  {
      saturationDrive:  1.5,
      saturationAmount: 1.0,
      saturationFreq:   5000,
    },
  },
  {
    name:  "3. phaser",
    label: "Phaser depth=0.15. Creates micro phase-shifts in formants — breaks the flat robotic pattern.",
    opts:  {
      phaserDepth: 0.15,
    },
  },
  {
    name:  "4. reverb",
    label: "Reverb: 12ms / 6% decay. Small room presence — dry voice = robotic, any room = more human.",
    opts:  {
      reverbMs:         12,
      reverbDecay:      0.06,
      reverbInputGain:  0.8,
      reverbOutputGain: 0.88,
    },
  },
  {
    name:  "5. exciter-phaser",
    label: "Exciter + phaser combined. Harmonic richness AND micro-variation together.",
    opts:  {
      saturationDrive:  1.5,
      saturationAmount: 1.0,
      saturationFreq:   5000,
      phaserDepth:      0.15,
    },
  },
  {
    name:  "6. all-three",
    label: "Exciter + phaser + reverb. Full anti-robotic stack — the kitchen sink.",
    opts:  {
      saturationDrive:  1.5,
      saturationAmount: 1.0,
      saturationFreq:   5000,
      phaserDepth:      0.15,
      reverbMs:         12,
      reverbDecay:      0.06,
      reverbInputGain:  0.8,
      reverbOutputGain: 0.88,
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
    if (PLAY) console.info(`\n[tuning] Focus: robotic voice\n[tuning] Phrase: "${PHRASE}"\n`);
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
