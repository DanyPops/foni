/**
 * pipeline/audio-test-utils.ts — backward-compatible re-export barrel.
 *
 * All imports have been split into focused modules:
 *   audio-utils.ts    — WAV parsing + DSP primitives (production)
 *
 * This barrel exists so existing test files keep working without changes.
 * New code should import directly from the relevant module.
 */

export * from "./audio-utils.ts";

// ─── Test signal generators (restored from deleted test-signals.ts) ───────────
/**
 * pipeline/test-signals.ts — Synthetic audio signal generators for tests.
 *
 * Test-only code. Never imported by production modules.
 * Provides deterministic WAV buffers for DSP unit tests and voice-quality tests.
 */

// ─── WAV header helper ────────────────────────────────────────────────────────

function writeWavHeader(buf: Buffer, sampleRate: number, numSamples: number): void {
  const dataSize = numSamples * 2;
  buf.write("RIFF", 0);  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write("WAVE", 8);  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);  buf.writeUInt16LE(1,  20);  // PCM
  buf.writeUInt16LE(1,  22);                               // mono
  buf.writeUInt32LE(sampleRate,     24);
  buf.writeUInt32LE(sampleRate * 2, 28);
  buf.writeUInt16LE(2, 32);   buf.writeUInt16LE(16, 34);  // 16-bit
  buf.write("data", 36);  buf.writeUInt32LE(dataSize, 40);
}

// ─── Signal generators ────────────────────────────────────────────────────────

/**
 * Generate a WAV buffer containing a pure sine wave at freqHz.
 * Output: 16-bit signed PCM, mono.
 */
export function generateSineWav(
  freqHz:       number,
  durationSecs: number,
  sampleRate    = 22050,
  amplitude     = 0.7,
): Buffer {
  const numSamples = Math.floor(sampleRate * durationSecs);
  const buf        = Buffer.alloc(44 + numSamples * 2);
  writeWavHeader(buf, sampleRate, numSamples);
  for (let i = 0; i < numSamples; i++) {
    const s = Math.sin(2 * Math.PI * freqHz * i / sampleRate) * amplitude;
    buf.writeInt16LE(Math.round(s * 32767), 44 + i * 2);
  }
  return buf;
}

/**
 * Generate a WAV buffer containing white noise (uniform random samples).
 * Useful for testing broadband filters and HNR calculations.
 */
export function generateNoiseWav(
  durationSecs: number,
  sampleRate    = 22050,
  amplitude     = 0.5,
): Buffer {
  const numSamples = Math.floor(sampleRate * durationSecs);
  const buf        = Buffer.alloc(44 + numSamples * 2);
  writeWavHeader(buf, sampleRate, numSamples);
  for (let i = 0; i < numSamples; i++) {
    const s = (Math.random() * 2 - 1) * amplitude;
    buf.writeInt16LE(Math.round(s * 32767), 44 + i * 2);
  }
  return buf;
}

/**
 * Generate a WAV buffer containing a harmonic series at f0.
 * Simulates a voiced vowel: harmonics 1–N with 1/h amplitude rolloff.
 */
export function generateHarmonicWav(
  f0Hz:         number,
  durationSecs: number,
  harmonics     = 8,
  sampleRate    = 22050,
  amplitude     = 0.6,
): Buffer {
  const numSamples = Math.floor(sampleRate * durationSecs);
  const buf        = Buffer.alloc(44 + numSamples * 2);
  writeWavHeader(buf, sampleRate, numSamples);
  for (let i = 0; i < numSamples; i++) {
    let s = 0;
    for (let h = 1; h <= harmonics; h++) {
      s += Math.sin(2 * Math.PI * f0Hz * h * i / sampleRate) / h;
    }
    s *= amplitude;
    buf.writeInt16LE(Math.round(Math.max(-32767, Math.min(32767, s * 32767))), 44 + i * 2);
  }
  return buf;
}

/**
 * Generate a silent WAV buffer (all zeros).
 * Useful for testing silence-gate thresholds.
 */
export function generateSilentWav(durationSecs: number, sampleRate = 22050): Buffer {
  const numSamples = Math.floor(sampleRate * durationSecs);
  const buf        = Buffer.alloc(44 + numSamples * 2);
  writeWavHeader(buf, sampleRate, numSamples);
  return buf;   // data region already zeroed by alloc
}


// ─── Voice validation (restored from deleted voice-analysis.ts) ───────────────
import { frameNoiseRatio, bandEnergy } from './audio-utils.ts';

export interface CriterionResult {
  name:  string;
  pass:  boolean;
  value: number;
  ideal: string;
  score?: number;
  weight?: number;
}

export interface AudioAnalysis {
  rmsDb:           number;
  peakDb:          number;
  crestFactor:     number;
  spectralSlope:   number;
  spectralTilt:    number;
  presenceRatio:   number;
  hfRatio:         number;
  noiseFloorRatio: number;
  voicedRatio:     number;
}

export interface ValidationResult {
  passed:      boolean;
  score:       number;
  redFailures: string[];
  criteria:    CriterionResult[];
  analysis:    AudioAnalysis;
}

function gaussScore(value: number, ideal: number, sigma: number): number {
  return Math.exp(-0.5 * ((value - ideal) / sigma) ** 2);
}

import { parseWav as _pw } from './audio-utils.ts';
import type { WavData } from './audio-utils.ts';

export function validateVoiceBuffer(wav: Buffer, sampleRate: number): ValidationResult {
  const { samples } = _pw(wav);
  const rmsVal   = Math.sqrt(samples.reduce((s, v) => s + v*v, 0) / samples.length) || 1e-10;
  const rmsDb    = 20 * Math.log10(rmsVal);
  const peakVal  = samples.reduce((m, v) => Math.max(m, Math.abs(v)), 0) || 1e-10;
  const peakDb   = 20 * Math.log10(peakVal);
  const nfr      = frameNoiseRatio(samples, sampleRate);
  const pres     = bandEnergy(samples, sampleRate, 2000, 5000);
  const hf       = bandEnergy(samples, sampleRate, 8000, sampleRate / 2 - 1);
  const lo       = Math.max(1e-10, bandEnergy(samples, sampleRate, 80, 500));
  const hi       = Math.max(1e-10, bandEnergy(samples, sampleRate, 2000, 8000));
  const spectralSlope = 20 * Math.log10(hi / lo) / (Math.log10(4000) - Math.log10(280));
  const spectralTilt  = 20 * Math.log10(lo / hi); // positive = low-freq dominant (natural speech)

  const analysis: AudioAnalysis = {
    rmsDb, peakDb, crestFactor: peakDb - rmsDb,
    spectralSlope, spectralTilt, presenceRatio: pres,
    hfRatio: hf, noiseFloorRatio: nfr, voicedRatio: nfr < 0.05 ? 0.7 : 0.3,
  };

  const criteria: CriterionResult[] = [];
  const redFailures: string[] = [];
  const reds = [
    { name: "no-clip",         pass: peakDb      < -0.5,  value: peakDb,      ideal: "< -0.5 dBFS" },
    { name: "not-silent",      pass: rmsDb        > -40,   value: rmsDb,       ideal: "> -40 dBFS"  },
    { name: "tilt-sane",       pass: spectralTilt > -20,   value: spectralTilt,ideal: "> -20 dB"    },
    { name: "has-dynamics",    pass: peakDb - rmsDb > 3,   value: peakDb-rmsDb,ideal: "> 3 dB"      },
    { name: "no-static-noise", pass: nfr          < 0.02,  value: nfr,         ideal: "< 0.02"      },
  ];
  for (const r of reds) {
    criteria.push({ ...r });
    if (!r.pass) redFailures.push(r.name);
  }
  if (redFailures.length > 0) return { passed: false, score: 0, redFailures, criteria, analysis };

  // GREEN score (simplified)
  const score = gaussScore(spectralTilt, 13, 5) * 0.1 +
                gaussScore(peakDb - rmsDb, 17, 4) * 0.25 +
                gaussScore(pres, 0.30, 0.08) * 0.20 +
                gaussScore(rmsDb, -15, 3) * 0.10;
  return { passed: true, score, redFailures: [], criteria, analysis };
}
