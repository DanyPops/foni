/**
 * pipeline/audio-utils.ts — WAV parsing and DSP primitives.
 *
 * Production code. No test signal generators, no analysis logic.
 * No external dependencies.
 *
 * Imported by: voice-analysis.ts, voice-quality.ts, gap-scorer.ts, dsp.test.ts
 */

// ─── WAV parsing ──────────────────────────────────────────────────────────────

export interface WavData {
  samples:    Float32Array;  // normalised [-1, 1]
  sampleRate: number;
  channels:   number;
  bitDepth:   number;
}

/**
 * Parse a WAV buffer into Float32 samples.
 * Handles non-standard chunk ordering by scanning for the "data" marker.
 */
export function parseWav(wav: Buffer): WavData {
  if (wav.toString("ascii", 0, 4) !== "RIFF") {
    throw new Error("Not a RIFF WAV file");
  }

  const channels   = wav.readUInt16LE(22);
  const sampleRate = wav.readUInt32LE(24);
  const bitDepth   = wav.readUInt16LE(34);

  // Scan for "data" chunk (may not be at fixed offset 36)
  let dataOffset = 44;
  for (let i = 12; i < wav.length - 8; i++) {
    if (wav.toString("ascii", i, i + 4) === "data") {
      dataOffset = i + 8;
      break;
    }
  }

  const bytesPerSample = bitDepth / 8;
  const numSamples     = Math.floor((wav.length - dataOffset) / bytesPerSample);
  const samples        = new Float32Array(numSamples);

  for (let i = 0; i < numSamples; i++) {
    const offset = dataOffset + i * bytesPerSample;
    if (bitDepth === 16) {
      samples[i] = wav.readInt16LE(offset) / 32768;
    } else if (bitDepth === 32) {
      samples[i] = wav.readInt32LE(offset) / 2147483648;
    } else {
      samples[i] = (wav.readUInt8(offset) - 128) / 128;
    }
  }

  return { samples, sampleRate, channels, bitDepth };
}

// ─── DSP primitives ───────────────────────────────────────────────────────────

/** Root mean square amplitude — measures average loudness. */
export function rms(samples: Float32Array): number {
  let sum = 0;
  for (const s of samples) sum += s * s;
  return Math.sqrt(sum / samples.length);
}

/** Peak absolute sample level. 1.0 = clipping. */
export function peak(samples: Float32Array): number {
  let max = 0;
  for (const s of samples) max = Math.max(max, Math.abs(s));
  return max;
}

/**
 * Goertzel algorithm — measure energy at a specific frequency.
 *
 * Uses a fixed window size to avoid catastrophic floating-point cancellation
 * that occurs in the naive single-pass algorithm on long signals.
 *
 * @param samples    PCM samples normalised to [-1, 1]
 * @param freqHz     Target frequency to measure
 * @param sampleRate Sample rate of the audio
 * @param offset     Sample offset to start reading from (skip silence padding)
 * @param window     Max samples to analyse per chunk
 */
export function goertzel(
  samples:    Float32Array,
  freqHz:     number,
  sampleRate: number,
  offset      = 0,
  window      = 2048,
): number {
  const omega = 2 * Math.PI * freqHz / sampleRate;
  const coeff = 2 * Math.cos(omega);
  const start = Math.min(offset, samples.length);
  const end   = Math.min(start + window, samples.length);
  const N     = end - start;
  if (N <= 0) return 0;

  let s1 = 0, s2 = 0;
  for (let i = start; i < end; i++) {
    const s0 = samples[i]! + coeff * s1 - s2;
    s2 = s1; s1 = s0;
  }
  return Math.sqrt(Math.max(0, s1 * s1 + s2 * s2 - coeff * s1 * s2)) / N;
}

/**
 * Convert amplitude ratio to dB change.
 *   dbChange(input, output) < 0  →  output is quieter (filter cut)
 *   dbChange(input, output) > 0  →  output is louder  (filter boost)
 */
export function dbChange(inputAmp: number, outputAmp: number): number {
  if (inputAmp <= 0 || outputAmp <= 0) return -Infinity;
  return 20 * Math.log10(outputAmp / inputAmp);
}

/**
 * Spectral centroid — weighted average frequency.
 * Higher = brighter signal. Lower = duller/muddier.
 */
export function spectralCentroid(samples: Float32Array, sampleRate: number): number {
  const freqs = [80, 150, 250, 400, 600, 1000, 1500, 2500, 4000, 6000, 8000, 12000];
  let weightedSum = 0, totalWeight = 0;
  for (const f of freqs) {
    if (f > sampleRate / 2) break;
    const e = goertzel(samples, f, sampleRate);
    weightedSum += f * e;
    totalWeight += e;
  }
  return totalWeight > 0 ? weightedSum / totalWeight : 0;
}

/**
 * Measure energy in a frequency band by summing Goertzel at logarithmically
 * spaced frequencies within the band.
 */
export function bandEnergy(
  samples:    Float32Array,
  sampleRate: number,
  lowHz:      number,
  highHz:     number,
  steps       = 8,
): number {
  const logLow  = Math.log(lowHz);
  const logHigh = Math.log(highHz);
  let total = 0;
  for (let i = 0; i < steps; i++) {
    const f = Math.exp(logLow + (logHigh - logLow) * i / (steps - 1));
    if (f > sampleRate / 2) break;
    total += goertzel(samples, f, sampleRate);
  }
  return total / steps;
}

// ─── Noise floor ─────────────────────────────────────────────────────────────

/**
 * Frame-based noise floor ratio.
 * Splits audio into ~10ms frames, measures per-frame RMS, computes ratio of
 * the 10th-percentile (quietest) to 90th-percentile (loudest) frame.
 * Constant background static → high ratio; clean voice with silences → near 0.
 */
export function frameNoiseRatio(
  samples:    Float32Array,
  sampleRate: number,
  frameMs     = 10,
): number {
  const frameSize = Math.floor(sampleRate * frameMs / 1000);
  const frameRms: number[] = [];
  for (let i = 0; i + frameSize <= samples.length; i += frameSize) {
    frameRms.push(rms(samples.subarray(i, i + frameSize)));
  }
  if (frameRms.length < 10) return 0;
  frameRms.sort((a, b) => a - b);
  const floor  = frameRms[Math.floor(frameRms.length * 0.10)]!;
  const signal = frameRms[Math.floor(frameRms.length * 0.90)]!;
  return signal > 0 ? floor / signal : 0;
}

// ─── Test assertion helpers ───────────────────────────────────────────────────

/** Assert that `outputAmp` is at least `minCutDb` quieter than `inputAmp`. */
export function assertCut(
  inputAmp:  number,
  outputAmp: number,
  minCutDb:  number,
  label:     string,
): void {
  const actual = dbChange(inputAmp, outputAmp);
  if (actual >= minCutDb) {
    throw new Error(`${label}: expected ≤${minCutDb.toFixed(1)}dB, got ${actual.toFixed(1)}dB`);
  }
}

/** Assert that `outputAmp` is at least `minBoostDb` louder than `inputAmp`. */
export function assertBoost(
  inputAmp:   number,
  outputAmp:  number,
  minBoostDb: number,
  label:      string,
): void {
  const actual = dbChange(inputAmp, outputAmp);
  if (actual <= minBoostDb) {
    throw new Error(`${label}: expected ≥+${minBoostDb.toFixed(1)}dB, got ${actual.toFixed(1)}dB`);
  }
}
