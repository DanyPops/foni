/**
 * pipeline/wav-utils.ts — Minimal WAV I/O + DSP primitives for production code.
 *
 * These three functions are used by processors.ts and breath-injector.ts.
 * Kept here (not in pipeline/analysis/) because they are production dependencies,
 * not analysis-layer code.
 */

export interface WavData {
  samples:    Float32Array;
  sampleRate: number;
  channels:   number;
  bitDepth:   number;
}

export function parseWav(wav: Buffer): WavData {
  if (wav.toString("ascii", 0, 4) !== "RIFF") throw new Error("Not a RIFF WAV file");
  const channels   = wav.readUInt16LE(22);
  const sampleRate = wav.readUInt32LE(24);
  const bitDepth   = wav.readUInt16LE(34);
  let dataOffset   = 44;
  for (let i = 12; i < wav.length - 8; i++) {
    if (wav.toString("ascii", i, i + 4) === "data") { dataOffset = i + 8; break; }
  }
  const bps     = bitDepth / 8;
  const n       = Math.floor((wav.length - dataOffset) / bps);
  const samples = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const o = dataOffset + i * bps;
    samples[i] = bitDepth === 16 ? wav.readInt16LE(o) / 32768
               : bitDepth === 32 ? wav.readInt32LE(o) / 2147483648
               : (wav.readUInt8(o) - 128) / 128;
  }
  return { samples, sampleRate, channels, bitDepth };
}

export function rms(samples: Float32Array): number {
  let sum = 0;
  for (const s of samples) sum += s * s;
  return Math.sqrt(sum / samples.length);
}

export function peak(samples: Float32Array): number {
  let max = 0;
  for (const s of samples) max = Math.max(max, Math.abs(s));
  return max;
}
