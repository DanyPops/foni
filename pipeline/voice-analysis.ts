/**
 * pipeline/voice-analysis.ts — Acoustic analysis and tuning-gate validator.
 *
 * Production code. Analyses voice WAV buffers and scores them against
 * RED/GREEN quality criteria. Used by tuning-validate.test.ts and gap-scorer.ts.
 *
 * No test signal generators here — see test-signals.ts.
 */

import { parseWav, rms, peak, goertzel, frameNoiseRatio } from "./audio-utils.ts";

export { frameNoiseRatio } from "./audio-utils.ts";

// ─── Types ────────────────────────────────────────────────────────────────────

export interface AudioAnalysis {
  rmsDb:           number;   // overall level
  peakDb:          number;   // peak level (clip check)
  crestFactor:     number;   // peakDb − rmsDb (dynamics)
  spectralTilt:    number;   // 20*log10(p@200Hz / p@3kHz) — LF advantage dB
  presenceRatio:   number;   // power@2kHz / totalBandPower
  hfRatio:         number;   // (power@4kHz + power@8kHz) / totalBandPower
  lfRatio:         number;   // (power@200Hz + power@500Hz) / totalBandPower
  noiseFloorRatio: number;   // quietest-10% / loudest-90% frame RMS ratio
  spectralSlope:   number;   // dB/octave (linear regression) — natural ≈ −6
}

export interface CriterionResult {
  name:    string;
  pass:    boolean;
  value:   number;
  ideal:   string;
  score?:  number;
  weight?: number;
}

export interface ValidationResult {
  passed:      boolean;
  score:       number;
  analysis:    AudioAnalysis;
  criteria:    CriterionResult[];
  redFailures: string[];
}

// ─── Analysis ─────────────────────────────────────────────────────────────────

/** Analyse a synthesised voice WAV buffer. */
export function analyseVoiceBuffer(wav: Buffer, sampleRate: number): AudioAnalysis {
  const { samples } = parseWav(wav);

  const rmsLin  = rms(samples);
  const peakLin = peak(samples);
  const rmsDb_  = rmsLin  > 0 ? 20 * Math.log10(rmsLin)  : -Infinity;
  const peakDb_ = peakLin > 0 ? 20 * Math.log10(peakLin) : -Infinity;
  const crest   = isFinite(rmsDb_) && isFinite(peakDb_) ? peakDb_ - rmsDb_ : 0;

  const p200 = goertzel(samples, 200,  sampleRate);
  const p500 = goertzel(samples, 500,  sampleRate);
  const p1k  = goertzel(samples, 1000, sampleRate);
  const p2k  = goertzel(samples, 2000, sampleRate);
  const p3k  = goertzel(samples, 3000, sampleRate);
  const p4k  = goertzel(samples, 4000, sampleRate);
  const p8k  = goertzel(samples, 8000, sampleRate);

  const tilt = p3k > 0 ? 20 * Math.log10(p200 / p3k) : 20;

  // Spectral slope: linear regression of dB levels across 6 bands (dB/octave)
  const bandPowers = [p200, p500, p1k, p2k, p4k, p8k];
  const bandFreqs  = [200,  500,  1000, 2000, 4000, 8000];
  const logFreqs   = bandFreqs.map(f => Math.log2(f / 200));
  const dbs        = bandPowers.map(p => p > 0 ? 20 * Math.log10(p) : -80);
  const n = logFreqs.length;
  const meanX = logFreqs.reduce((a, b) => a + b, 0) / n;
  const meanY = dbs.reduce((a, b) => a + b, 0) / n;
  const slope = logFreqs.reduce((num, x, i) => num + (x - meanX) * (dbs[i]! - meanY), 0) /
                logFreqs.reduce((den, x) => den + (x - meanX) ** 2, 0);

  const total7 = p200 + p500 + p1k + p2k + p3k + p4k + p8k;
  return {
    rmsDb:           rmsDb_,
    peakDb:          peakDb_,
    crestFactor:     crest,
    spectralTilt:    tilt,
    spectralSlope:   slope,
    presenceRatio:   total7 > 0 ? p2k  / total7 : 0,
    hfRatio:         total7 > 0 ? (p4k + p8k) / total7 : 0,
    lfRatio:         total7 > 0 ? (p200 + p500) / total7 : 0,
    noiseFloorRatio: frameNoiseRatio(samples, sampleRate),
  };
}

// ─── Tuning gate validator ─────────────────────────────────────────────────────

function gaussScore(value: number, ideal: number, sigma: number): number {
  return Math.exp(-0.5 * ((value - ideal) / sigma) ** 2);
}

/** Validate a synthesised voice buffer against RED/GREEN quality criteria. */
export function validateVoiceBuffer(wav: Buffer, sampleRate: number): ValidationResult {
  const a = analyseVoiceBuffer(wav, sampleRate);
  const criteria: CriterionResult[] = [];
  const redFailures: string[] = [];

  // RED — drop if any fail
  const reds = [
    { name: "no-clip",         pass: a.peakDb          < -0.5,  value: a.peakDb,          ideal: "< −0.5 dBFS" },
    { name: "not-silent",      pass: a.rmsDb            > -40,   value: a.rmsDb,           ideal: "> −40 dBFS"  },
    { name: "tilt-sane",       pass: a.spectralTilt     > -20,   value: a.spectralTilt,    ideal: "> −20 dB"    },
    { name: "has-dynamics",    pass: a.crestFactor      > 3,     value: a.crestFactor,     ideal: "> 3 dB"      },
    { name: "no-static-noise", pass: a.noiseFloorRatio  < 0.02,  value: a.noiseFloorRatio, ideal: "< 0.02"      },
  ];
  for (const r of reds) {
    criteria.push({ ...r });
    if (!r.pass) redFailures.push(r.name);
  }
  if (redFailures.length > 0) {
    return { passed: false, score: 0, analysis: a, criteria, redFailures };
  }

  // GREEN — scored 0–1, weighted
  const greens = [
    { name: "spectral-slope", value: a.spectralSlope, ideal: "−6 dB/oct",      score: gaussScore(a.spectralSlope, -6,   3),    weight: 0.25 },
    { name: "spectral-tilt",  value: a.spectralTilt,  ideal: "8–18 dB",         score: gaussScore(a.spectralTilt,  13,   5),    weight: 0.10 },
    { name: "crest-factor",   value: a.crestFactor,   ideal: "15–20 dB",         score: gaussScore(a.crestFactor,   17,   4),    weight: 0.25 },
    { name: "presence-ratio", value: a.presenceRatio, ideal: "0.20–0.40",       score: gaussScore(a.presenceRatio, 0.30, 0.08), weight: 0.20 },
    { name: "rms-level",      value: a.rmsDb,         ideal: "−17 to −13 dBFS", score: gaussScore(a.rmsDb,         -15,  3),    weight: 0.10 },
    { name: "hf-containment", value: a.hfRatio,       ideal: "< 0.20",          score: Math.max(0, 1 - a.hfRatio / 0.20),      weight: 0.10 },
  ];

  let weightedScore = 0;
  for (const g of greens) {
    criteria.push({ name: g.name, pass: true, value: g.value, ideal: g.ideal, score: g.score, weight: g.weight });
    weightedScore += g.score * g.weight;
  }

  return { passed: true, score: weightedScore, analysis: a, criteria, redFailures: [] };
}
