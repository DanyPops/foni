/**
 * Tuning round 4 — how far can we push each anti-robotic lever?
 *
 * Baseline v3: de-harsh + punch + exciter(1.5@5kHz) + phaser(0.15) + reverb(12ms/6%)
 *
 * Each variant pushes ONE lever harder to find the sweet spot before
 * it tips over into "over-processed" or "echo-y" territory.
 *
 *   npm run listen:1   # baseline v3
 *   npm run listen:2   # exciter harder (drive=2.5)
 *   npm run listen:3   # exciter lower freq (4kHz — hits more of the midrange)
 *   npm run listen:4   # phaser deeper (0.3)
 *   npm run listen:5   # reverb longer (20ms / 8% decay)
 *   npm run listen:6   # all levers pushed (2.5 drive + 0.3 phaser + 20ms reverb)
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

const CONFIGS: Array<{ name: string; label: string; opts: Partial<SmoothingOptions> }> = [
  {
    name:  "1. baseline",
    label: "v3 — exciter(1.5@5kHz) + phaser(0.15) + reverb(12ms/6%). Reference.",
    opts:  {},
  },
  {
    name:  "2. exciter-harder",
    label: "Exciter drive=2.5 (was 1.5). More harmonic richness — does it warm up or distort?",
    opts:  { saturationDrive: 2.5 },
  },
  {
    name:  "3. exciter-lower",
    label: "Exciter freq=4kHz (was 5kHz). Excites more of the midrange — more body or more harsh?",
    opts:  { saturationFreq: 4000 },
  },
  {
    name:  "4. phaser-deeper",
    label: "Phaser depth=0.3 (was 0.15). More phase movement — more organic or too wobbly?",
    opts:  { phaserDepth: 0.3 },
  },
  {
    name:  "5. reverb-longer",
    label: "Reverb 20ms / 8% decay (was 12ms/6%). More room — more natural or too echo-y?",
    opts:  { reverbMs: 20, reverbDecay: 0.08 },
  },
  {
    name:  "6. all-pushed",
    label: "All three levers pushed: drive=2.5 + phaser=0.3 + reverb=20ms/8%.",
    opts:  {
      saturationDrive: 2.5,
      saturationFreq:  4000,
      phaserDepth:     0.3,
      reverbMs:        20,
      reverbDecay:     0.08,
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
