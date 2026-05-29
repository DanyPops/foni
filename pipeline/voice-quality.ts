/**
 * Voice quality metrics — HNR, CPP, jitter, shimmer, F0 contour analysis.
 *
 * All algorithms are self-contained with no external dependencies.
 * Based on Praat methodology and COVAREP research toolkit conventions.
 *
 * Primary use: mathematical roboticness assessment of synthesised Russian TTS.
 * Focus heuristics:
 *   - Jitter near 0 → espeak generates perfectly regular periods (synthetic)
 *   - HNR > 30 dB   → signal too pure for natural speech
 *   - F0 stdDev < 5 Hz → monotone (robotic)
 *   - slopeSemi < 1 for "?" → missing question intonation
 */

import { parseWav } from "./audio-utils.ts";

// ─── F0 search range ──────────────────────────────────────────────────────────

const MIN_F0_HZ = 75;   // below = unvoiced / bass rumble
const MAX_F0_HZ = 500;  // above = noise peak / sibilant

// ─── FFT (Cooley-Tukey, iterative radix-2) ───────────────────────────────────

/** Smallest power of 2 ≥ n. */
function nextPow2(n: number): number {
  let p = 1;
  while (p < n) p <<= 1;
  return p;
}

/**
 * In-place FFT (Cooley-Tukey, iterative radix-2).
 * re and im must have the same length, which must be a power of 2.
 */
export function fft(re: Float64Array, im: Float64Array): void {
  const N = re.length;

  // bit-reversal permutation
  for (let i = 1, j = 0; i < N; i++) {
    let bit = N >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      [re[i], re[j]] = [re[j], re[i]];
      [im[i], im[j]] = [im[j], im[i]];
    }
  }

  // butterfly
  for (let len = 2; len <= N; len <<= 1) {
    const ang = -2 * Math.PI / len;
    const wRe = Math.cos(ang);
    const wIm = Math.sin(ang);
    for (let i = 0; i < N; i += len) {
      let curRe = 1, curIm = 0;
      for (let j = 0; j < len >> 1; j++) {
        const uRe = re[i + j],          uIm = im[i + j];
        const h   = i + j + (len >> 1);
        const vRe = re[h] * curRe - im[h] * curIm;
        const vIm = re[h] * curIm + im[h] * curRe;
        re[i + j] = uRe + vRe;   im[i + j] = uIm + vIm;
        re[h]     = uRe - vRe;   im[h]     = uIm - vIm;
        const nRe = curRe * wRe - curIm * wIm;
        curIm     = curRe * wIm + curIm * wRe;
        curRe     = nRe;
      }
    }
  }
}

// ─── Window ───────────────────────────────────────────────────────────────────

/** Hann (raised-cosine) window, length N. */
function hannWin(N: number): Float64Array {
  const w = new Float64Array(N);
  for (let i = 0; i < N; i++) w[i] = 0.5 * (1 - Math.cos(2 * Math.PI * i / (N - 1)));
  return w;
}

// ─── Autocorrelation ──────────────────────────────────────────────────────────

/**
 * Normalized autocorrelation of a windowed frame.
 * r[0] = 1.0, r[k] = Σ(x[n]·x[n+k]) / Σ(x[n]²).
 */
function normalizedAC(frame: Float64Array): Float64Array {
  const N = frame.length;
  const r = new Float64Array(N);
  let power = 0;
  for (let i = 0; i < N; i++) power += frame[i] * frame[i];
  if (power < 1e-14) { r[0] = 1; return r; }
  for (let k = 0; k < N; k++) {
    let sum = 0;
    for (let i = 0; i < N - k; i++) sum += frame[i] * frame[i + k];
    r[k] = sum / power;
  }
  return r;
}

// ─── F0 extraction ────────────────────────────────────────────────────────────

export interface F0Frame {
  timeMs:   number;    // frame centre in ms
  f0Hz:     number;    // 0 if unvoiced
  voiced:   boolean;
  hnrDb:    number;    // harmonics-to-noise ratio (−∞ if unvoiced)
  rms:      number;    // frame RMS amplitude
  peakCorr: number;    // normalised autocorrelation peak (0–1)
}

/**
 * Extract per-frame F0 and HNR via normalised autocorrelation (SHR-style).
 *
 * @param samples    Float32 PCM, normalised −1..1
 * @param sampleRate audio sample rate
 * @param frameMs    analysis frame size (default 25 ms)
 * @param hopMs      hop between frames (default 10 ms)
 */
export function extractF0(
  samples:    Float32Array,
  sampleRate: number,
  frameMs   = 25,
  hopMs     = 10,
): F0Frame[] {
  const frameSize = Math.floor(sampleRate * frameMs / 1000);
  const hopSize   = Math.floor(sampleRate * hopMs  / 1000);
  const minLag    = Math.floor(sampleRate / MAX_F0_HZ);
  const maxLag    = Math.min(Math.floor(sampleRate / MIN_F0_HZ), frameSize - 1);
  const hann      = hannWin(frameSize);
  const frames: F0Frame[] = [];

  for (let start = 0; start + frameSize <= samples.length; start += hopSize) {
    const timeMs = (start + frameSize / 2) / sampleRate * 1000;

    // apply Hann window
    const frame = new Float64Array(frameSize);
    let rmsSum = 0;
    for (let i = 0; i < frameSize; i++) {
      frame[i] = samples[start + i] * hann[i];
      rmsSum  += frame[i] * frame[i];
    }
    const frameRms = Math.sqrt(rmsSum / frameSize);

    // silence gate
    if (frameRms < 0.001) {
      frames.push({ timeMs, f0Hz: 0, voiced: false, hnrDb: -Infinity, rms: frameRms, peakCorr: 0 });
      continue;
    }

    // autocorrelation peak search — collect ALL local maxima in [minLag, maxLag]
    const r = normalizedAC(frame);
    const peaks: Array<{ lag: number; val: number }> = [];
    for (let k = minLag + 1; k < maxLag; k++) {
      const prev = r[k - 1] ?? 0, cur = r[k] ?? 0, next = r[k + 1] ?? 0;
      if (cur > prev && cur > next && cur > 0.25) peaks.push({ lag: k, val: cur });
    }
    if (peaks.length === 0) {
      // No local max found — fall back to global max in range
      let gl = minLag, gv = r[minLag] ?? 0;
      for (let k = minLag + 1; k <= maxLag; k++) {
        if ((r[k] ?? 0) > gv) { gv = r[k]!; gl = k; }
      }
      peaks.push({ lag: gl, val: gv });
    }

    // Octave-preference: among peaks, prefer the LARGEST lag (lowest F0)
    // that has AC ≥ 0.85 × the strongest peak. This strongly biases toward
    // the fundamental over harmonics (which appear at smaller lags).
    const bestVal = Math.max(...peaks.map(p => p.val));
    const candidates = peaks
      .filter(p => p.val >= 0.85 * bestVal)
      .sort((a, b) => b.lag - a.lag);   // largest lag first = lowest F0 first
    const chosen = candidates[0] ?? peaks.sort((a, b) => b.val - a.val)[0]!;
    let peakLag = chosen.lag;
    let peakVal = chosen.val;

    // parabolic interpolation for sub-sample F0 accuracy
    const iLag = Math.round(peakLag);
    if (iLag > 0 && iLag < r.length - 1) {
      const alpha = r[iLag - 1]!, beta = r[iLag]!, gamma = r[iLag + 1]!;
      const denom = alpha - 2 * beta + gamma;
      if (Math.abs(denom) > 1e-10) peakLag = iLag + 0.5 * (alpha - gamma) / denom;
    }

    const voiced = peakVal > 0.30;
    const f0Hz   = voiced ? sampleRate / peakLag : 0;
    // HNR from normalised AC: r = H/(H+N) → HNR = r/(1−r)
    const hnrDb  = voiced && peakVal < 1
      ? 10 * Math.log10(peakVal / (1 - peakVal))
      : voiced ? 30 : -Infinity;

    frames.push({ timeMs, f0Hz, voiced, hnrDb, rms: frameRms, peakCorr: peakVal });
  }

  return frames;
}

// ─── Jitter & Shimmer ─────────────────────────────────────────────────────────

/**
 * Local jitter (RAP): mean absolute consecutive period deviation / mean period.
 * Natural speech < 1 %; perfectly synthetic (espeak) → ~0 %.
 */
export function computeJitter(frames: F0Frame[]): number {
  const voiced = frames.filter(f => f.voiced && f.f0Hz > 0);
  if (voiced.length < 3) return NaN;
  const periods = voiced.map(f => 1000 / f.f0Hz); // ms
  let delta = 0;
  for (let i = 1; i < periods.length; i++) delta += Math.abs(periods[i]! - periods[i - 1]!);
  const mean = periods.reduce((a, b) => a + b, 0) / periods.length;
  return mean > 0 ? delta / (periods.length - 1) / mean : NaN;
}

/**
 * Local shimmer (dB-amplitude): mean absolute consecutive RMS deviation / mean RMS.
 * Natural speech < 5 %; perfectly synthetic → near 0 %.
 */
export function computeShimmer(frames: F0Frame[]): number {
  const voiced = frames.filter(f => f.voiced && f.f0Hz > 0);
  if (voiced.length < 3) return NaN;
  const amps = voiced.map(f => f.rms);
  let delta = 0;
  for (let i = 1; i < amps.length; i++) delta += Math.abs(amps[i]! - amps[i - 1]!);
  const mean = amps.reduce((a, b) => a + b, 0) / amps.length;
  return mean > 0 ? delta / (amps.length - 1) / mean : NaN;
}

// ─── F0 contour statistics ────────────────────────────────────────────────────

export interface F0ContourStats {
  meanHz:      number;   // mean F0 of voiced frames
  stdDevHz:    number;   // overall F0 variation (< 5 Hz = monotone)
  meanDeltaHz: number;   // mean frame-to-frame F0 change
  stdDevDelta: number;   // variation in deltas
  maxDeltaHz:  number;   // largest single jump
  slopeSemi:   number;   // net contour slope in semitones (+ = rising)
  voicedCount: number;
  totalCount:  number;
  voicedRatio: number;
}

/** Compute summary statistics over the F0 contour. */
export function f0ContourStats(frames: F0Frame[]): F0ContourStats {
  const voiced = frames.filter(f => f.voiced && f.f0Hz > 0);

  if (voiced.length < 2) {
    return {
      meanHz: 0, stdDevHz: 0, meanDeltaHz: 0, stdDevDelta: 0, maxDeltaHz: 0,
      slopeSemi: 0, voicedCount: voiced.length, totalCount: frames.length, voicedRatio: 0,
    };
  }

  const f0s    = voiced.map(f => f.f0Hz);
  const meanHz = f0s.reduce((a, b) => a + b, 0) / f0s.length;
  const stdDev = Math.sqrt(f0s.reduce((s, f) => s + (f - meanHz) ** 2, 0) / f0s.length);

  const deltas = f0s.slice(1).map((f, i) => Math.abs(f - f0s[i]!));
  const meanD  = deltas.reduce((a, b) => a + b, 0) / deltas.length;
  const sdD    = Math.sqrt(deltas.reduce((s, d) => s + (d - meanD) ** 2, 0) / deltas.length);

  // net slope: semitones from first voiced to last voiced frame
  const slopeSemi = f0s.length > 1
    ? 12 * Math.log2(f0s[f0s.length - 1]! / f0s[0]!)
    : 0;

  return {
    meanHz, stdDevHz: stdDev,
    meanDeltaHz: meanD, stdDevDelta: sdD,
    maxDeltaHz: deltas.length > 0 ? Math.max(...deltas) : 0,
    slopeSemi,
    voicedCount: voiced.length, totalCount: frames.length,
    voicedRatio: voiced.length / frames.length,
  };
}

// ─── CPP (Cepstral Peak Prominence) ──────────────────────────────────────────

/**
 * Compute CPP for a single Hann-windowed frame.
 *
 * Algorithm (simplified Hillenbrand/Milenkovic):
 *   1. FFT → log power spectrum
 *   2. IFFT of log spectrum → real cepstrum
 *   3. Find peak in quefrency range for F0
 *   4. Fit linear regression baseline over same quefrency range
 *   5. CPP = peak − baseline
 *
 * Interpretation:
 *   > 14 dB: clearly voiced and well-structured
 *   > 25 dB: suspiciously high — likely synthetic (espeak)
 *   < 10 dB: noisy / unvoiced
 */
export function computeCPP(frame: Float64Array, sampleRate: number): number {
  const N    = frame.length;
  const fftN = nextPow2(N);
  const hann = hannWin(N);

  // FFT of windowed frame
  const re = new Float64Array(fftN);
  const im = new Float64Array(fftN);
  for (let i = 0; i < N; i++) re[i] = frame[i] * hann[i];
  fft(re, im);

  // log power spectrum → real cepstrum via second FFT
  const cRe = new Float64Array(fftN);
  for (let i = 0; i < fftN; i++) {
    cRe[i] = Math.log(Math.max(re[i] * re[i] + im[i] * im[i], 1e-20));
  }
  const cIm = new Float64Array(fftN);
  fft(cRe, cIm);

  // quefrency range corresponding to MIN_F0_HZ–MAX_F0_HZ
  const minQ = Math.floor(sampleRate / MAX_F0_HZ);
  const maxQ = Math.min(Math.floor(sampleRate / MIN_F0_HZ), Math.floor(fftN / 2));

  // find cepstral peak
  let peakQ = minQ, peakVal = cRe[minQ]! / fftN;
  for (let q = minQ + 1; q <= maxQ; q++) {
    const v = cRe[q]! / fftN;
    if (v > peakVal) { peakVal = v; peakQ = q; }
  }

  // linear regression baseline over [minQ, maxQ]
  const n = maxQ - minQ + 1;
  let sumX = 0, sumY = 0, sumXX = 0, sumXY = 0;
  for (let q = minQ; q <= maxQ; q++) {
    const v = cRe[q]! / fftN;
    sumX += q; sumY += v; sumXX += q * q; sumXY += q * v;
  }
  const slope  = (n * sumXY - sumX * sumY) / (n * sumXX - sumX * sumX + 1e-20);
  const interc = (sumY - slope * sumX) / n;
  const baseline = slope * peakQ + interc;

  return peakVal - baseline;
}

// ─── Full voice quality assessment ───────────────────────────────────────────

export interface VoiceQualityMetrics {
  f0Stats:     F0ContourStats;
  hnrDbMean:   number;   // mean HNR of voiced frames
  hnrDbMax:    number;   // max HNR (synthetic peaks very high)
  cppMean:     number;   // mean CPP across voiced frames  (dB-equivalent units)
  jitter:      number;   // ratio; NaN if < 3 voiced frames
  shimmer:     number;   // ratio; NaN if < 3 voiced frames
  voicedRatio: number;
  frameCount:  number;
}

/**
 * Compute all voice quality metrics from a raw WAV buffer.
 * Assumes mono 16-bit PCM (standard espeak / RVC output).
 */
export function computeVoiceQuality(
  wav:        Buffer,
  sampleRate: number,
  frameMs   = 25,
  hopMs     = 10,
): VoiceQualityMetrics {
  const { samples } = parseWav(wav);
  const frames  = extractF0(samples, sampleRate, frameMs, hopMs);
  const voiced  = frames.filter(f => f.voiced);

  const hnrMean = voiced.length > 0
    ? voiced.reduce((s, f) => s + f.hnrDb, 0) / voiced.length
    : -Infinity;
  const hnrMax  = voiced.length > 0
    ? Math.max(...voiced.map(f => f.hnrDb))
    : -Infinity;

  // CPP: compute per voiced frame, use frame from audio samples
  const fSize   = Math.floor(sampleRate * frameMs / 1000);
  const hopSize = Math.floor(sampleRate * hopMs  / 1000);
  let cppSum = 0, cppCount = 0;
  for (let fi = 0; fi < frames.length; fi++) {
    if (!frames[fi]!.voiced) continue;
    const start = fi * hopSize;
    if (start + fSize > samples.length) continue;
    const rawFrame = new Float64Array(fSize);
    for (let i = 0; i < fSize; i++) rawFrame[i] = samples[start + i]!;
    cppSum += computeCPP(rawFrame, sampleRate);
    cppCount++;
  }

  return {
    f0Stats:     f0ContourStats(frames),
    hnrDbMean:   hnrMean,
    hnrDbMax:    hnrMax,
    cppMean:     cppCount > 0 ? cppSum / cppCount : 0,
    jitter:      computeJitter(frames),
    shimmer:     computeShimmer(frames),
    voicedRatio: frames.length > 0 ? voiced.length / frames.length : 0,
    frameCount:  frames.length,
  };
}

// ─── Roboticness score ────────────────────────────────────────────────────────

export interface RoboticFactor {
  name:    string;
  value:   string;
  penalty: number;   // 0–100 weighted contribution
  note:    string;
}

export interface RoboticnessAssessment {
  score:          number;          // 0 = natural, 100 = fully mechanical
  label:          "Natural" | "Slight" | "Moderate" | "Robotic" | "Mechanical";
  factors:        RoboticFactor[];
  recommendation: string;
}

/**
 * Composite roboticness score from voice quality metrics.
 *
 * Weights (sum to 100):
 *   F0 monotonicity  30 — espeak generates nearly constant F0
 *   Jitter deficit   25 — synthetic voice has near-zero jitter
 *   HNR excess       20 — too pure = TTS artefact
 *   F0 slope/contour 15 — question intonation should be rising
 *   Shimmer deficit  10 — amplitude too stable
 */
export function roboticnessScore(m: VoiceQualityMetrics): RoboticnessAssessment {
  const factors: RoboticFactor[] = [];

  // ── F0 monotonicity (weight 30) ──────────────────────────────────────────
  // stdDev < 5 Hz = completely flat; natural conversational = 20–50 Hz
  const monoRaw = Math.max(0, 1 - m.f0Stats.stdDevHz / 20) * 30;
  factors.push({
    name:    "F0 monotonicity",
    value:   `σ = ${m.f0Stats.stdDevHz.toFixed(1)} Hz`,
    penalty: monoRaw,
    note:    monoRaw > 20 ? "Pitch barely varies — sounds mechanical" : "Good pitch variation",
  });

  // ── Jitter deficit (weight 25) ────────────────────────────────────────────
  // Human < 1 %; espeak ≈ 0 %
  const jit = isNaN(m.jitter) ? 0 : m.jitter;
  const jitPenalty = Math.max(0, 1 - jit / 0.008) * 25;
  factors.push({
    name:    "Jitter deficit",
    value:   isNaN(m.jitter) ? "N/A" : `${(m.jitter * 100).toFixed(3)} %`,
    penalty: jitPenalty,
    note:    jitPenalty > 15 ? "Periods too regular — synthetic fingerprint" : "Natural jitter",
  });

  // ── HNR excess (weight 20) ────────────────────────────────────────────────
  // Natural voiced speech: 15–25 dB; TTS often > 30 dB
  const hnrPenalty = Math.min(20, Math.max(0, (m.hnrDbMean - 20) / 15) * 20);
  factors.push({
    name:    "HNR excess",
    value:   `${m.hnrDbMean.toFixed(1)} dB (max ${m.hnrDbMax.toFixed(1)} dB)`,
    penalty: hnrPenalty,
    note:    m.hnrDbMean > 28 ? "Signal too pure — overtone structure too clean" : "Normal harmonic noise mix",
  });

  // ── F0 contour / question intonation (weight 15) ──────────────────────────
  // For "Понял, брателло?" → should have rising intonation (slopeSemi > 2)
  // Also reward delta variation (smooth contour = less robotic)
  const slopePenalty = Math.max(0, 1 - m.f0Stats.slopeSemi / 3) * 8;
  const deltaVarPenalty = Math.max(0, 1 - m.f0Stats.stdDevDelta / 5) * 7;
  factors.push({
    name:    "F0 contour",
    value:   `slope ${m.f0Stats.slopeSemi.toFixed(1)} st, Δ-σ ${m.f0Stats.stdDevDelta.toFixed(1)} Hz`,
    penalty: slopePenalty + deltaVarPenalty,
    note:    m.f0Stats.slopeSemi < 1 ? "No rising question intonation" : "Rising intonation present",
  });

  // ── Shimmer deficit (weight 10) ───────────────────────────────────────────
  // Human < 5 %; TTS < 1 %
  const shim = isNaN(m.shimmer) ? 0 : m.shimmer;
  const shimPenalty = Math.max(0, 1 - shim / 0.03) * 10;
  factors.push({
    name:    "Shimmer deficit",
    value:   isNaN(m.shimmer) ? "N/A" : `${(m.shimmer * 100).toFixed(2)} %`,
    penalty: shimPenalty,
    note:    shimPenalty > 6 ? "Amplitude too stable between cycles" : "Normal amplitude variation",
  });

  const totalScore = Math.min(100, factors.reduce((s, f) => s + f.penalty, 0));
  const label =
    totalScore < 15 ? "Natural"   :
    totalScore < 30 ? "Slight"    :
    totalScore < 50 ? "Moderate"  :
    totalScore < 70 ? "Robotic"   : "Mechanical";

  const worst = [...factors].sort((a, b) => b.penalty - a.penalty)[0]!;
  const recommendation = `Primary factor: ${worst.name} — ${worst.note}`;

  return { score: totalScore, label, factors, recommendation };
}
