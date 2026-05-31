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
