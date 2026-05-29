#!/usr/bin/env node
/**
 * scripts/analyse.mjs — Mathematical roboticness assessment of a synthesised phrase.
 *
 * Usage:
 *   node scripts/analyse.mjs [phrase] [--lang ru] [--no-rvc]
 *
 * Default phrase: "Понял, брателло?"  (the most robotic-sounding phrase in our corpus)
 *
 * Pipeline: espeak → [SmoothingProcessor] → [RVC] → voice-quality metrics + ASCII plots
 *
 * No external npm packages — all DSP is in pipeline/voice-quality.ts and pipeline/spectrogram.ts.
 */

import { spawnSync, execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

// ─── Args ─────────────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
const flagNoRvc  = args.includes("--no-rvc");
const flagLang   = args.find(a => a.startsWith("--lang="))?.split("=")[1] ?? "ru";
const phraseArg  = args.filter(a => !a.startsWith("--"))[0];
const PHRASE     = phraseArg ?? "Понял, брателло?";

// ─── ESM compat: load TypeScript modules via tsx register ─────────────────────
// We use tsx (already in devDependencies) to import .ts files from an .mjs script.

import { register } from "node:module";
// tsx/esm registers itself when invoked; if not, fall back to dynamic require.
// Simplest: we run this script via `tsx scripts/analyse.mjs` so imports work.

import { parseWav }         from "../pipeline/audio-test-utils.ts";
import { computeVoiceQuality, roboticnessScore, extractF0, f0ContourStats } from "../pipeline/voice-quality.ts";
import { asciiSpectrogram, asciiPSD, asciiF0Contour } from "../pipeline/spectrogram.ts";

// ─── Synthesise via espeak ────────────────────────────────────────────────────

function synthesise(phrase, lang = "ru") {
  const tmpDir = mkdtempSync(join(tmpdir(), "foni-analyse-"));
  const outWav = join(tmpDir, "out.wav");

  const result = spawnSync("espeak-ng", [
    "-v", lang,
    "-s", "145",   // speed
    "-p", "50",    // pitch
    "-a", "200",   // amplitude
    "-w", outWav,
    phrase,
  ], { encoding: "buffer" });

  if (result.error) throw new Error(`espeak-ng failed: ${result.error.message}`);
  if (!existsSync(outWav)) throw new Error("espeak-ng produced no output file");

  return readFileSync(outWav);
}

// ─── Run RVC (optional) ───────────────────────────────────────────────────────

async function runRvc(wavBuf) {
  try {
    const resp = await fetch("http://localhost:5050/convert", {
      method:  "POST",
      headers: { "Content-Type": "application/octet-stream", "X-Pitch": "-2" },
      body:    wavBuf,
      signal:  AbortSignal.timeout(30_000),
    });
    if (!resp.ok) return null;
    return Buffer.from(await resp.arrayBuffer());
  } catch {
    return null;
  }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

console.log(`\n${"═".repeat(72)}`);
console.log(`  Foni Roboticness Analyser`);
console.log(`  Phrase : "${PHRASE}"`);
console.log(`  Lang   : ${flagLang}`);
console.log(`${"═".repeat(72)}\n`);

// 1. Synthesise
process.stdout.write("▶ Synthesising via espeak-ng … ");
const espeakWav = synthesise(PHRASE, flagLang);
const espeakParsed = parseWav(espeakWav);
console.log(`OK  (${espeakParsed.samples.length} samples @ ${espeakParsed.sampleRate} Hz, ${(espeakWav.length / 1024).toFixed(1)} kB)`);

// 2. Optionally process through RVC
let analysisWav  = espeakWav;
let analysisLabel = "espeak-ng";

if (!flagNoRvc) {
  process.stdout.write("▶ Sending to RVC (bandit model) … ");
  const rvcWav = await runRvc(espeakWav);
  if (rvcWav) {
    analysisWav   = rvcWav;
    analysisLabel = "espeak-ng → RVC";
    console.log(`OK  (${(rvcWav.length / 1024).toFixed(1)} kB)`);
  } else {
    console.log("SKIP (RVC unavailable — analysing espeak output only)");
  }
}

// 3. Parse WAV for analysis
const parsed     = parseWav(analysisWav);
const { samples, sampleRate } = parsed;
const totalMs    = samples.length / sampleRate * 1000;

console.log(`\n── Source: ${analysisLabel} ──`);
console.log(`   Duration: ${totalMs.toFixed(0)} ms  |  Channels: ${parsed.channels}  |  BitDepth: ${parsed.bitDepth}`);

// 4. F0 contour
console.log("\n── F0 Contour (pitch track) ────────────────────────────────────────────");
const f0Frames = extractF0(samples, sampleRate);
const f0Stats  = f0ContourStats(f0Frames);

console.log(asciiF0Contour(f0Frames, totalMs, {
  width: 70, height: 14,
  title: `F0 contour — "${PHRASE}"`,
}));

console.log(`
   Mean F0   : ${f0Stats.meanHz.toFixed(1)} Hz
   StdDev    : ${f0Stats.stdDevHz.toFixed(1)} Hz  (< 5 Hz = robotic monotone)
   Mean Δ    : ${f0Stats.meanDeltaHz.toFixed(1)} Hz / frame
   Δ-σ       : ${f0Stats.stdDevDelta.toFixed(1)} Hz  (step regularity — low = quantised)
   Max jump  : ${f0Stats.maxDeltaHz.toFixed(1)} Hz
   Slope     : ${f0Stats.slopeSemi.toFixed(2)} semitones  (+ = rising, expected for "?")
   Voiced    : ${f0Stats.voicedCount} / ${f0Stats.totalCount} frames (${(f0Stats.voicedRatio * 100).toFixed(0)} %)`);

// 5. Spectrogram
console.log("\n── Spectrogram (100 Hz – 4 kHz) ────────────────────────────────────────");
console.log(asciiSpectrogram(samples, sampleRate, {
  width: 70, height: 18,
  minHz: 100, maxHz: 4000, gamma: 0.4,
  title: `Spectrogram — "${PHRASE}"`,
}));

// 6. PSD
console.log("\n── Power Spectral Density ───────────────────────────────────────────────");
console.log(asciiPSD(samples, sampleRate, {
  points: 32, width: 56, minHz: 80, maxHz: 6000,
  title: `PSD — "${PHRASE}"`,
}));

// 7. Voice quality metrics
console.log("\n── Voice Quality Metrics ────────────────────────────────────────────────");
const metrics = computeVoiceQuality(analysisWav, sampleRate);
const assess  = roboticnessScore(metrics);

const rows = [
  ["Metric",           "Value",                                  "Reference"],
  ["─────────────────","──────────────────────────────","─────────────────"],
  ["HNR (mean)",       `${metrics.hnrDbMean.toFixed(1)} dB`,     "15–25 dB natural"],
  ["HNR (max)",        `${metrics.hnrDbMax.toFixed(1)} dB`,      "> 30 dB = TTS artefact"],
  ["CPP (mean)",       `${metrics.cppMean.toFixed(4)}`,          "> 0 = voiced; high = pure"],
  ["Jitter",           isNaN(metrics.jitter) ? "N/A" : `${(metrics.jitter*100).toFixed(4)} %`, "< 1 % natural; ~0 % synthetic"],
  ["Shimmer",          isNaN(metrics.shimmer) ? "N/A" : `${(metrics.shimmer*100).toFixed(3)} %`, "< 5 % natural; ~0 % synthetic"],
  ["Voiced ratio",     `${(metrics.voicedRatio*100).toFixed(0)} %`, "> 60 % for continuous speech"],
];
for (const [m, v, r] of rows) {
  console.log(`   ${m.padEnd(16)} ${v.padEnd(30)} ${r}`);
}

// 8. Roboticness assessment
console.log("\n── Roboticness Assessment ───────────────────────────────────────────────");
const bar = (penalty) => "█".repeat(Math.round(penalty / 2)).padEnd(50, "░");
for (const f of assess.factors) {
  console.log(`   ${f.name.padEnd(20)} ${f.value.padEnd(28)} penalty=${f.penalty.toFixed(1)}`);
  console.log(`   ${"".padEnd(20)} ${bar(f.penalty)}`);
}

const scoreBar = "█".repeat(Math.round(assess.score / 2)).padEnd(50, "░");
console.log(`\n   ${"TOTAL SCORE".padEnd(20)} ${assess.score.toFixed(1)} / 100  [${assess.label}]`);
console.log(`   ${"".padEnd(20)} ${scoreBar}`);
console.log(`\n   ${assess.recommendation}`);

// 9. Interpretation
console.log("\n── Interpretation ───────────────────────────────────────────────────────");
if (f0Stats.stdDevHz < 5) {
  console.log("   ⚠  MONOTONE: F0 barely varies. espeak generates stepped/quantised pitch.");
  console.log("      Fix: post-process F0 with a natural intonation model or SSML prosody tags.");
}
if (f0Stats.slopeSemi < 1) {
  console.log("   ⚠  MISSING QUESTION INTONATION: '?' should produce rising pitch contour.");
  console.log("      espeak often fails to produce smooth rising F0 for Russian questions.");
}
if (isFinite(metrics.hnrDbMean) && metrics.hnrDbMean > 25) {
  console.log("   ⚠  OVER-PURE: HNR > 25 dB indicates unnaturally clean harmonic structure.");
  console.log("      Fix: add breathiness (noise) component or apply subtle pitch jitter.");
}
if (!isNaN(metrics.jitter) && metrics.jitter < 0.002) {
  console.log("   ⚠  ZERO JITTER: Period variation < 0.2 % — synthetic fingerprint.");
  console.log("      Fix: vibratoFreq/vibratoDepth in SmoothingProcessor breaks regularity.");
}
console.log(`\n${"═".repeat(72)}\n`);
