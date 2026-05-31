/**
 * sidorovich.e2e.test.ts — Phrase-matched gap test: trader1a.wav vs our synthesis.
 *
 * Unlike baseline.e2e.test.ts (averaged 3-file tensor, different phrases),
 * this test synthesises the exact text spoken in trader1a.wav and scores
 * our output against that single file's acoustic fingerprint directly.
 *
 * Requires: espeak-ng, RVC server at localhost:5050.
 */

import { describe, it, expect, beforeAll } from "vitest";
import { readFileSync, existsSync, mkdtempSync } from "node:fs";
import { spawnSync }                             from "node:child_process";
import { join }                                  from "node:path";
import { tmpdir }                                from "node:os";
import { ESPEAK_BASE_WPM }                       from "./backends/espeak.ts";

import {
  computeGap,
  formatGapTable,
} from "./pipeline/analysis/gap-scorer.ts";
import type { TargetTensor }             from "./pipeline/analysis/gap-scorer.ts";
import { analyseVoiceBuffer }            from "./pipeline/analysis/voice-analysis.ts";
import { computeVoiceQuality }           from "./pipeline/analysis/voice-quality.ts";
import { SmoothingProcessor, RVCProcessor } from "./pipeline/processors.ts";
import { DEFAULT_CONFIG }                from "./core/config.ts";

// ─── Constants ────────────────────────────────────────────────────────────────

const SAMPLE_RATE   = 22050;
const PHRASE        = "Подойди-ка, надо тебе ситуацию прояснить.";
const ORIGINAL_WAV  = "baseline/stalker/wav/sidorovich/trader1a.wav";
const GAP_CEILING_PCT = 80;

// ─── Build a TargetTensor from a single WAV ───────────────────────────────────

function wavToTensor(wav: Buffer, label: string): TargetTensor {
  const ac = analyseVoiceBuffer(wav, SAMPLE_RATE);
  const vq = computeVoiceQuality(wav, SAMPLE_RATE);
  return {
    _description: label,
    _sources:     [label],
    voice: {
      f0MeanHz:        vq.f0Stats.meanHz,
      f0StdDevHz:      vq.f0Stats.stdDevHz,
      f0SlopeSemi:     vq.f0Stats.slopeSemi,
      f0DeltaSigmaHz:  vq.f0Stats.stdDevDelta,
      voicedRatio:     vq.voicedRatio,
      hnrDbMean:       vq.hnrDbMean,
      hnrDbMax:        vq.hnrDbMax,
      cppMean:         vq.cppMean,
      jitter:          isNaN(vq.jitter)  ? 0.008 : vq.jitter,
      shimmer:         isNaN(vq.shimmer) ? 0.040 : vq.shimmer,
    },
    spectral: {
      rmsDb:           ac.rmsDb,
      spectralSlope:   ac.spectralSlope,
      spectralTilt:    ac.spectralTilt,
      crestFactor:     ac.crestFactor,
      presenceRatio:   ac.presenceRatio,
      hfRatio:         ac.hfRatio,
      lfRatio:         ac.lfRatio,
      noiseFloorRatio: ac.noiseFloorRatio,
    },
    roboticness: { targetScore: 0 },
  };
}

// ─── Pipeline helpers ─────────────────────────────────────────────────────────

function synthesiseEspeak(phrase: string): Buffer {
  const dir = mkdtempSync(join(tmpdir(), "foni-sidorovich-"));
  const out = join(dir, "out.wav");
  const r   = spawnSync(
    "espeak-ng",
    ["-v", "ru", "-s", String(Math.round(ESPEAK_BASE_WPM * 1.15)), "-p", "50", "-a", "200", "-w", out, phrase],
    { encoding: "buffer" },
  );
  if (r.error || !existsSync(out)) {
    throw new Error(`espeak-ng failed: ${r.error?.message ?? "no output"}`);
  }
  return readFileSync(out);
}

async function synthesiseFull(phrase: string): Promise<Buffer> {
  const raw  = synthesiseEspeak(phrase);
  const proc = new SmoothingProcessor(new RVCProcessor(DEFAULT_CONFIG.rvcUrl));
  return proc.process(raw);
}

// ─── Fixtures ─────────────────────────────────────────────────────────────────

let tensor: TargetTensor;
let result: ReturnType<typeof computeGap>;

beforeAll(async () => {
  if (!existsSync(ORIGINAL_WAV)) {
    throw new Error(`Reference WAV not found: ${ORIGINAL_WAV}`);
  }

  tensor = wavToTensor(readFileSync(ORIGINAL_WAV), "trader1a — Sidorovich (SoC)");

  const synth = await synthesiseFull(PHRASE);
  const ac    = analyseVoiceBuffer(synth, SAMPLE_RATE);
  const vq    = computeVoiceQuality(synth, SAMPLE_RATE);
  result      = computeGap(PHRASE, ac, vq, tensor);
}, 120_000);

// ─── Tests ────────────────────────────────────────────────────────────────────

it("gap table snapshot — trader1a vs our synthesis", () => {
  expect(formatGapTable(result)).toMatchSnapshot();
});

it(`mean gap stays below ${GAP_CEILING_PCT}%`, () => {
  const { meanGapPct } = result;
  expect(meanGapPct).toBeLessThan(GAP_CEILING_PCT);
});

it("no dimension at 100% gap", () => {
  const maxed = result.rows.filter(r => r.gapPct >= 100);
  if (maxed.length > 0) {
    throw new Error(
      `Dimensions at 100% gap:\n` +
      maxed.map(r => `  ${r.metric}: actual ${r.actual} vs target ${r.target}`).join("\n"),
    );
  }
});
