/**
 * test/audio.ts — shared audio utilities for tests.
 *
 * Re-exports the canonical WAV DSP from pipeline/analysis/audio-utils.ts
 * and adds test-specific conveniences (sine generator, validity checks).
 */

export {
  parseWav, rms, peak, dbChange,
  type WavData,
} from "../pipeline/analysis/audio-utils.ts";

// ─── Test-specific generators ─────────────────────────────────────────────────

/**
 * Generate a minimal valid 16-bit mono WAV buffer containing a sine wave.
 * `amplitude` 0..1, `freq` 0 = silence.
 */
export function generateSineWav(
  freq:       number,
  amplitude:  number,
  sampleRate: number,
  durationS:  number,
): Buffer {
  const n   = Math.floor(sampleRate * durationS);
  const buf = Buffer.alloc(44 + n * 2);

  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + n * 2, 4);
  buf.write("WAVE", 8);
  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28);
  buf.writeUInt16LE(2, 32);
  buf.writeUInt16LE(16, 34);
  buf.write("data", 36);
  buf.writeUInt32LE(n * 2, 40);

  for (let i = 0; i < n; i++) {
    const s   = freq > 0 ? Math.sin(2 * Math.PI * freq * i / sampleRate) * amplitude : 0;
    const s16 = Math.max(-32768, Math.min(32767, Math.round(s * 32767)));
    buf.writeInt16LE(s16, 44 + i * 2);
  }
  return buf;
}

export function silenceWav(sampleRate: number, durationMs: number): Buffer {
  return generateSineWav(0, 0, sampleRate, durationMs / 1000);
}

// ─── Test assertions ──────────────────────────────────────────────────────────

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
