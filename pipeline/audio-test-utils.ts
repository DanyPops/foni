/**
 * Audio test utilities — WAV generation, parsing, and acoustic assertions.
 *
 * No external dependencies. All DSP math is self-contained.
 * Designed for testing SmoothingProcessor filter behaviour.
 */

// ─── WAV generation ───────────────────────────────────────────────────────────

/**
 * Generate a WAV buffer containing a pure sine wave at freqHz.
 * Output: 16-bit signed PCM, mono, at sampleRate.
 */
export function generateSineWav(
  freqHz:      number,
  durationSecs: number,
  sampleRate  = 22050,
  amplitude   = 0.7,    // 0–1, headroom below clipping
): Buffer {
  const numSamples = Math.floor(sampleRate * durationSecs);
  const dataSize   = numSamples * 2;
  const buf        = Buffer.alloc(44 + dataSize);

  // RIFF header
  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write("WAVE", 8);
  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1,          20);  // PCM
  buf.writeUInt16LE(1,          22);  // mono
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28);
  buf.writeUInt16LE(2,          32);  // block align
  buf.writeUInt16LE(16,         34);  // bits per sample
  buf.write("data", 36);
  buf.writeUInt32LE(dataSize,   40);

  for (let i = 0; i < numSamples; i++) {
    const s = Math.sin(2 * Math.PI * freqHz * i / sampleRate) * amplitude;
    buf.writeInt16LE(Math.round(s * 32767), 44 + i * 2);
  }

  return buf;
}

/**
 * Generate a WAV buffer containing white noise (uniform random samples).
 * Useful for testing broadband filters.
 */
export function generateNoiseWav(
  durationSecs: number,
  sampleRate  = 22050,
  amplitude   = 0.5,
): Buffer {
  const numSamples = Math.floor(sampleRate * durationSecs);
  const dataSize   = numSamples * 2;
  const buf        = Buffer.alloc(44 + dataSize);

  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write("WAVE", 8);
  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1,          20);
  buf.writeUInt16LE(1,          22);
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28);
  buf.writeUInt16LE(2,          32);
  buf.writeUInt16LE(16,         34);
  buf.write("data", 36);
  buf.writeUInt32LE(dataSize,   40);

  for (let i = 0; i < numSamples; i++) {
    const s = (Math.random() * 2 - 1) * amplitude;
    buf.writeInt16LE(Math.round(s * 32767), 44 + i * 2);
  }

  return buf;
}

// ─── WAV parsing ─────────────────────────────────────────────────────────────

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

  const channels  = wav.readUInt16LE(22);
  const sampleRate = wav.readUInt32LE(24);
  const bitDepth  = wav.readUInt16LE(34);

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

// ─── Acoustic measurements ────────────────────────────────────────────────────

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
 * (s1² + s2² - coeff·s1·s2 becomes numerically unstable when s1, s2 >> 1.)
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
  const omega  = 2 * Math.PI * freqHz / sampleRate;
  const coeff  = 2 * Math.cos(omega);
  const start  = Math.min(offset, samples.length);
  const end    = Math.min(start + window, samples.length);
  const N      = end - start;
  if (N <= 0) return 0;

  let s1 = 0, s2 = 0;
  for (let i = start; i < end; i++) {
    const s0 = samples[i] + coeff * s1 - s2;
    s2 = s1;
    s1 = s0;
  }

  const power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
  return Math.sqrt(Math.max(0, power)) / N;
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
 * Computed via simple DFT approximation over logarithmic frequency bands.
 */
export function spectralCentroid(samples: Float32Array, sampleRate: number): number {
  const freqs = [80, 150, 250, 400, 600, 1000, 1500, 2500, 4000, 6000, 8000, 12000];
  let weightedSum = 0;
  let totalWeight = 0;
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

// ─── Assertion helpers ────────────────────────────────────────────────────────

/** Assert that `outputAmp` is at least `minCutDb` quieter than `inputAmp`. */
export function assertCut(
  inputAmp:  number,
  outputAmp: number,
  minCutDb:  number,
  label:     string,
): void {
  const actual = dbChange(inputAmp, outputAmp);
  if (actual >= minCutDb) {
    throw new Error(
      `${label}: expected ≤${minCutDb.toFixed(1)}dB change, got ${actual.toFixed(1)}dB`,
    );
  }
}

/** Assert that `outputAmp` is at least `minBoostDb` louder than `inputAmp`. */
export function assertBoost(
  inputAmp:  number,
  outputAmp: number,
  minBoostDb: number,
  label:     string,
): void {
  const actual = dbChange(inputAmp, outputAmp);
  if (actual <= minBoostDb) {
    throw new Error(
      `${label}: expected ≥+${minBoostDb.toFixed(1)}dB change, got ${actual.toFixed(1)}dB`,
    );
  }
}
