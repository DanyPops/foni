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
  { slug: "baseline",       opts: {} },
  { slug: "exciter-harder", opts: { saturationDrive: 2.5 } },
  { slug: "exciter-lower",  opts: { saturationFreq: 4000 } },
  { slug: "phaser-deeper",  opts: { phaserDepth: 0.3 } },
  { slug: "reverb-longer",  opts: { reverbMs: 20, reverbDecay: 0.08 } },
  { slug: "all-pushed",     opts: { saturationDrive: 2.5, saturationFreq: 4000, phaserDepth: 0.3, reverbMs: 20, reverbDecay: 0.08 } },
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
