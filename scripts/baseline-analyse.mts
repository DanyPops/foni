/**
 * scripts/baseline-analyse.mts
 *
 * Analyse the 3 STALKER reference WAV files, print per-file metrics,
 * then average them into a desired-state tensor → baseline/target.json
 *
 * Run: npx tsx scripts/baseline-analyse.mts
 */

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { computeVoiceQuality, roboticnessScore } from "../pipeline/analysis/voice-quality.ts";
import { analyseVoiceBuffer }                    from "../pipeline/analysis/audio-test-utils.ts";

const SAMPLE_RATE = 22050;
const BASE        = "./baseline/stalker/wav";

const FILES = [
  {
    id:    "sidorovich-trader1a",
    path:  `${BASE}/sidorovich/trader1a.wav`,
    label: "Sidorovich greeting (SoC)",
  },
  {
    id:    "cherevatenko-barmen-greet",
    path:  `${BASE}/cherevatenko/zat_a2_stalker_barmen_greeting_1.wav`,
    label: "Barmen greeting (CoP)",
  },
  {
    id:    "cherevatenko-barmen-fare",
    path:  `${BASE}/cherevatenko/zat_a2_stalker_barmen_farewell_1.wav`,
    label: "Barmen farewell (CoP)",
  },
] as const;

// ─── Per-file analysis ────────────────────────────────────────────────────────

type FileResult = {
  id:    string;
  label: string;
  vq:    ReturnType<typeof computeVoiceQuality>;
  ac:    ReturnType<typeof analyseVoiceBuffer>;
  rob:   ReturnType<typeof roboticnessScore>;
};

const results: FileResult[] = [];

for (const f of FILES) {
  const wav = readFileSync(f.path);
  const vq  = computeVoiceQuality(wav, SAMPLE_RATE);
  const ac  = analyseVoiceBuffer(wav, SAMPLE_RATE);
  const rob = roboticnessScore(vq);

  results.push({ id: f.id, label: f.label, vq, ac, rob });

  const row = (name: string, val: string, note = "") =>
    console.log(`  ${name.padEnd(18)} ${val.padEnd(20)} ${note}`);

  console.log(`\n${"═".repeat(66)}`);
  console.log(`  ${f.label}`);
  console.log(`${"─".repeat(66)}`);
  row("F0 mean",        `${vq.f0Stats.meanHz.toFixed(1)} Hz`);
  row("F0 stdDev",      `${vq.f0Stats.stdDevHz.toFixed(1)} Hz`,       "< 5 Hz = robotic monotone");
  row("F0 slope",       `${vq.f0Stats.slopeSemi.toFixed(2)} semitones`);
  row("F0 Δ-σ",         `${vq.f0Stats.stdDevDelta.toFixed(1)} Hz`,    "step regularity");
  row("Voiced ratio",   `${(vq.voicedRatio * 100).toFixed(0)} %`);
  row("HNR mean",       `${vq.hnrDbMean.toFixed(1)} dB`,              "15–25 dB = natural");
  row("HNR max",        `${vq.hnrDbMax.toFixed(1)} dB`,               "> 30 = TTS artefact");
  row("CPP mean",       `${vq.cppMean.toFixed(4)}`);
  row("Jitter",         isNaN(vq.jitter)  ? "N/A" : `${(vq.jitter  * 100).toFixed(3)} %`, "< 1% = natural");
  row("Shimmer",        isNaN(vq.shimmer) ? "N/A" : `${(vq.shimmer * 100).toFixed(3)} %`, "< 5% = natural");
  row("RMS",            `${ac.rmsDb.toFixed(1)} dBFS`);
  row("Spectral slope", `${ac.spectralSlope.toFixed(2)} dB/oct`,      "−6 = natural voice");
  row("Spectral tilt",  `${ac.spectralTilt.toFixed(1)} dB`);
  row("Crest factor",   `${ac.crestFactor.toFixed(1)} dB`,            "15–20 = natural dynamics");
  row("Noise floor",    `${ac.noiseFloorRatio.toFixed(4)}`);
  console.log(`  ${"─".repeat(64)}`);
  console.log(`  Roboticness     ${rob.score.toFixed(1)}/100  [${rob.label}]`);
  console.log(`  ↳ ${rob.recommendation}`);
}

// ─── Desired state tensor (average across 3 files) ────────────────────────────

const avg = (pick: (r: FileResult) => number) =>
  results.reduce((s, r) => s + pick(r), 0) / results.length;

const tensor = {
  _description: "Desired state tensor — averaged from 3 STALKER studio recordings",
  _sources:     FILES.map(f => f.id),
  voice: {
    f0MeanHz:        avg(r => r.vq.f0Stats.meanHz),
    f0StdDevHz:      avg(r => r.vq.f0Stats.stdDevHz),
    f0SlopeSemi:     avg(r => r.vq.f0Stats.slopeSemi),
    f0DeltaSigmaHz:  avg(r => r.vq.f0Stats.stdDevDelta),
    voicedRatio:     avg(r => r.vq.voicedRatio),
    hnrDbMean:       avg(r => r.vq.hnrDbMean),
    hnrDbMax:        avg(r => r.vq.hnrDbMax),
    cppMean:         avg(r => r.vq.cppMean),
    jitter:          avg(r => isNaN(r.vq.jitter)  ? 0.008 : r.vq.jitter),
    shimmer:         avg(r => isNaN(r.vq.shimmer) ? 0.040 : r.vq.shimmer),
  },
  spectral: {
    rmsDb:           avg(r => r.ac.rmsDb),
    spectralSlope:   avg(r => r.ac.spectralSlope),
    spectralTilt:    avg(r => r.ac.spectralTilt),
    crestFactor:     avg(r => r.ac.crestFactor),
    presenceRatio:   avg(r => r.ac.presenceRatio),
    hfRatio:         avg(r => r.ac.hfRatio),
    lfRatio:         avg(r => r.ac.lfRatio),
    noiseFloorRatio: avg(r => r.ac.noiseFloorRatio),
  },
  roboticness: {
    targetScore: avg(r => r.rob.score),
  },
};

mkdirSync("./baseline", { recursive: true });
writeFileSync("./baseline/target.json", JSON.stringify(tensor, null, 2));

console.log(`\n${"═".repeat(66)}`);
console.log(`  DESIRED STATE TENSOR  (average of ${results.length} files)`);
console.log(`${"─".repeat(66)}`);
const v = tensor.voice;
const s = tensor.spectral;
console.log(`  F0 mean        ${v.f0MeanHz.toFixed(1)} Hz`);
console.log(`  F0 stdDev      ${v.f0StdDevHz.toFixed(1)} Hz      ← target variation floor`);
console.log(`  Jitter         ${(v.jitter * 100).toFixed(3)} %`);
console.log(`  Shimmer        ${(v.shimmer * 100).toFixed(3)} %`);
console.log(`  HNR mean       ${v.hnrDbMean.toFixed(1)} dB`);
console.log(`  Spectral slope ${s.spectralSlope.toFixed(2)} dB/oct`);
console.log(`  Crest factor   ${s.crestFactor.toFixed(1)} dB`);
console.log(`  Roboticness    ${tensor.roboticness.targetScore.toFixed(1)}/100`);
console.log(`${"─".repeat(66)}`);
console.log(`  Written → baseline/target.json`);
console.log(`${"═".repeat(66)}\n`);
