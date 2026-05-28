/**
 * Pipeline E2E showcase — EspeakBackend (RU) → RVCProcessor (bandit) → SystemPlayer
 *
 * Uses the existing production classes directly — no ad-hoc curl or scripts.
 *
 * Prerequisites:
 *   espeak-ng with Russian voice    (dnf install espeak-ng)
 *   RVC server running              (podman start foni-rvc)
 *
 * Run (silent):
 *   RVC_URL=http://127.0.0.1:5050 npm test -- pipeline.e2e
 *
 * Run (with audio — hear every phrase):
 *   FONI_PLAY=1 RVC_URL=http://127.0.0.1:5050 npm test -- pipeline.e2e
 */

import { describe, it, expect, beforeAll } from "vitest";
import { EspeakBackend }  from "./backends/espeak.ts";
import { RVCProcessor, SmoothingProcessor } from "./pipeline/processors.ts";
import { SystemPlayer }   from "./pipeline/player.ts";
import { SpeakFacade }    from "./pipeline/speak-facade.ts";
import { IdentityTranslator } from "./pipeline/translators.ts";

const RVC_URL = process.env.RVC_URL  ?? "http://127.0.0.1:5050";
const PLAY    = process.env.FONI_PLAY === "1";

// ─── Showcase phrases ─────────────────────────────────────────────────────────
//
// Hand-picked for maximum bandit-voice impact. Sources:
//   S.T.A.L.K.E.R. bandit dialogue
//   Soviet film catchphrases (The Irony of Fate, Caucasian Captive, Operation Y)
//
// Text is already Russian — IdentityTranslator passes it straight to espeak.

const PHRASES = [
  // ── STALKER bandits ───────────────────────────────────────────────────────
  {
    label: "cheeki breeki",
    text:  "Ну-ка, чики-брики и в дамке! Понял, брателло?",
  },
  {
    label: "bandit threat",
    text:  "Выходи по-хорошему, иначе хуже будет, ёб твою мать.",
  },
  {
    label: "bandit philosophy",
    text:  "Зона, она такая — сегодня живёшь, блядь, а завтра уже нет. Вот природа.",
  },
  {
    label: "bandit banter",
    text:  "Ну ты что, тупой? Я же говорил — обходи слева, пиздец!",
  },

  // ── Soviet film classics ──────────────────────────────────────────────────
  {
    label: "irony of fate — fish aspic",
    text:  "Какая гадость эта ваша заливная рыба! Ну и ну!",
  },
  {
    label: "caucasian captive — student athlete",
    text:  "Студентка, комсомолка, спортсменка и просто красавица. Ого!",
  },
  {
    label: "operation Y — write it down",
    text:  "Будьте добры помедленнее, я записываю! Эх, куда вы так торопитесь.",
  },

  // ── Pure Russian internet vibes ───────────────────────────────────────────
  {
    label: "existential mat",
    text:  "Господи, как скучно мы живём! Мы перестали делать большие и хорошие глупости, блядь.",
  },
  {
    label: "zone wisdom",
    text:  "Зона не прощает слабых. Ни хуя себе, это точно, братан.",
  },
] as const;

// ─── Helpers ─────────────────────────────────────────────────────────────────

async function isRvcReachable(): Promise<boolean> {
  try {
    const r = await fetch(`${RVC_URL}/params`, { signal: AbortSignal.timeout(2000) });
    return r.ok;
  } catch { return false; }
}

function buildFacade(play: boolean): SpeakFacade {
  return new SpeakFacade(
    new IdentityTranslator(),                           // text already Russian
    new EspeakBackend("ru"),                            // Russian espeak voice
    new SmoothingProcessor(new RVCProcessor(RVC_URL)),  // pad → RVC → smooth
    play ? new SystemPlayer() : new NullPlayer(),
    { voice: "ru", speed: 1.15 },
  );
}

/** Drop-in Player that discards audio — used in silent test runs. */
class NullPlayer {
  detected() { return "null" as const; }
  async play(_buf: Buffer): Promise<void> {}
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("Pipeline E2E showcase: EspeakBackend(RU) → RVCProcessor(bandit)", () => {
  let skip = false;
  let facade!: SpeakFacade;

  beforeAll(async () => {
    const espeakOk = await new EspeakBackend("ru").isAvailable();
    const rvcOk    = await isRvcReachable();

    if (!espeakOk) { console.warn("[e2e] espeak-ng not found — skipping"); skip = true; }
    if (!rvcOk)    { console.warn(`[e2e] RVC not reachable at ${RVC_URL} — skipping`); skip = true; }
    if (skip) return;

    facade = buildFacade(PLAY);

    const params = await fetch(`${RVC_URL}/params`).then(r => r.json()) as Record<string, unknown>;
    console.info(`[e2e] RVC model: ${params.current_model}  pitch: ${params.f0up_key}`);
    if (PLAY) console.info("[e2e] FONI_PLAY=1 — playing each phrase");
  });

  for (const { label, text } of PHRASES) {
    it(`[${label}]`, async () => {
      if (skip) return;

      const log: string[] = [];
      await facade.speak(text, msg => {
        log.push(msg);
        console.info(`[e2e] ${msg}`);
      });

      // Assert the full pipeline ran — all three stages logged
      expect(log.some(m => m.startsWith("backend=espeak"))).toBe(true);
      expect(log.some(m => m.includes("synthesized"))).toBe(true);
      expect(log.some(m => m.includes("played"))).toBe(true);
    }, 120_000);
  }
});
