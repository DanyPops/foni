/**
 * test/audio.ts — shared audio generation and assertion helpers.
 *
 * Single source of truth for WAV fixtures and acoustic assertions.
 */

// ─── WAV generation ───────────────────────────────────────────────────────────

/**
 * Generate a minimal valid 16-bit mono WAV buffer.
 * `amplitude` 0..1, `freq` 0 = silence.
 */
export function generateSineWav(
  freq:       number,
  amplitude:  number,
  sampleRate: number,
  durationS:  number,
): Buffer {
  const n       = Math.floor(sampleRate * durationS);
  const buf     = Buffer.alloc(44 + n * 2);

  // RIFF header
  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + n * 2, 4);
  buf.write("WAVE", 8);
  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);       // chunk size
  buf.writeUInt16LE(1, 20);        // PCM
  buf.writeUInt16LE(1, 22);        // mono
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28);
  buf.writeUInt16LE(2, 32);        // block align
  buf.writeUInt16LE(16, 34);       // bits per sample
  buf.write("data", 36);
  buf.writeUInt32LE(n * 2, 40);

  // Samples
  for (let i = 0; i < n; i++) {
    const t    = i / sampleRate;
    const s    = freq > 0 ? Math.sin(2 * Math.PI * freq * t) * amplitude : 0;
    const s16  = Math.round(s * 32767);
    buf.writeInt16LE(Math.max(-32768, Math.min(32767, s16)), 44 + i * 2);
  }

  return buf;
}

/** Silence WAV of given duration. */
export function silenceWav(sampleRate: number, durationMs: number): Buffer {
  return generateSineWav(0, 0, sampleRate, durationMs / 1000);
}

// ─── WAV parsing ──────────────────────────────────────────────────────────────

export interface ParsedWav {
  sampleRate: number;
  samples:    Float32Array;
  durationS:  number;
}

export function parseWav(buf: Buffer): ParsedWav {
  const sampleRate = buf.readUInt32LE(24);
  const dataLen    = buf.readUInt32LE(40);
  const n          = dataLen / 2;
  const samples    = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    samples[i] = buf.readInt16LE(44 + i * 2) / 32768;
  }
  return { sampleRate, samples, durationS: n / sampleRate };
}

// ─── Acoustic assertions ──────────────────────────────────────────────────────

export function rms(samples: Float32Array): number {
  if (samples.length === 0) return 0;
  const sum = samples.reduce((s, v) => s + v * v, 0);
  return Math.sqrt(sum / samples.length);
}

export function rmsDb(samples: Float32Array): number {
  const r = rms(samples);
  return r > 0 ? 20 * Math.log10(r) : -Infinity;
}

export function isNonSilent(buf: Buffer, thresholdDb = -60): boolean {
  try {
    const { samples } = parseWav(buf);
    return rmsDb(samples) > thresholdDb;
  } catch { return false; }
}

export function isValidWav(buf: Buffer): boolean {
  return buf.length >= 44 &&
         buf.toString("ascii", 0, 4) === "RIFF" &&
         buf.toString("ascii", 8, 12) === "WAVE";
}
