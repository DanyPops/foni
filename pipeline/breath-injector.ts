/**
 * pipeline/breath-injector.ts — Splice synthetic breath sounds at sentence boundaries.
 *
 * Breath sounds are inserted in the WAV domain AFTER espeak synthesis,
 * BEFORE RVC, so the voice model processes them as part of the input.
 *
 * Strategy:
 *   1. Scan the synthesised WAV for silence gaps >= SILENCE_GATE_MS
 *   2. At each gap, splice in a synthetic breath WAV (120ms)
 *   3. The breath fills the gap with realistic air intake texture
 *
 * The synthetic breath is generated once via foni-synth POST /breath (Rust bandpass noise) and
 * cached in memory — no file I/O on the hot path.
 *
 * Research basis:
 *   - PubMed 7759655: breath intake improves listener recall of synthetic speech
 *   - US9508338B1 (Google): TTS breath insertion at phrase boundaries
 *   - KTH 2020: breathing events at long-phrase / clause-initial positions
 */

import { parseWav, rms } from "./analysis/audio-utils.ts";
import { getLogger }     from "../core/logger.ts";
import type { AudioProcessor } from "./interfaces.ts";

// ─── Constants ────────────────────────────────────────────────────────────────

/** Minimum silence gap duration (ms) before we consider inserting a breath. */
const SILENCE_GATE_MS    = 80;

/** RMS amplitude threshold below which a frame counts as "silent". */
const SILENCE_THRESHOLD  = 0.005;   // ≈ −46 dBFS

/** Duration of the synthesised breath sound (ms). */
const BREATH_DURATION_MS = 120;

// ─── Breath WAV generation via foni-synth /breath (Rust, no ffmpeg) ─────────

/**
 * Synthesise a breath-intake WAV via POST /breath on foni-synth.
 * Falls back to silence if the server is unreachable.
 */
async function synthesiseBreath(
  sampleRate: number,
  durationMs: number,
  synthUrl:   string,
): Promise<Buffer> {
  try {
    const resp = await fetch(`${synthUrl}/breath`, {
      method:  "POST",
      headers: { "Content-Type": "application/json" },
      body:    JSON.stringify({ duration_ms: durationMs, sample_rate: sampleRate }),
      signal:  AbortSignal.timeout(5_000),
    });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const { audio_data } = await resp.json() as { audio_data: string };
    return Buffer.from(audio_data, "base64");
  } catch (e: any) {
    const msg = `foni-synth /breath unreachable: ${e?.message}`;
    if (process.env.FONI_REQUIRE_DSP === "1") throw new Error(msg);
    getLogger().warn("BreathInjector", `${msg} — using silence`);
    // Silence fallback: minimal valid WAV
    const n   = Math.floor(sampleRate * durationMs / 1000);
    const buf = Buffer.alloc(44 + n * 2);
    buf.write("RIFF", 0); buf.writeUInt32LE(36 + n * 2, 4);
    buf.write("WAVE", 8); buf.write("fmt ", 12);
    buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22);
    buf.writeUInt32LE(sampleRate, 24); buf.writeUInt32LE(sampleRate * 2, 28);
    buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34);
    buf.write("data", 36); buf.writeUInt32LE(n * 2, 40);
    return buf;
  }
}

// ─── Silence detection ────────────────────────────────────────────────────────

interface SilenceGap {
  startSample: number;
  endSample:   number;
  durationMs:  number;
}

/**
 * Find silence gaps in a Float32 sample array.
 * Returns gaps sorted by start position.
 */
function findSilenceGaps(
  samples:    Float32Array,
  sampleRate: number,
  gateMs:     number     = SILENCE_GATE_MS,
  threshold:  number     = SILENCE_THRESHOLD,
): SilenceGap[] {
  const frameMs   = 10;
  const frameSize = Math.floor(sampleRate * frameMs / 1000);
  const gateMs_   = gateMs;

  const gaps: SilenceGap[] = [];
  let silenceStart: number | null = null;

  for (let i = 0; i + frameSize <= samples.length; i += frameSize) {
    const frameRms = rms(samples.subarray(i, i + frameSize));
    const silent   = frameRms < threshold;

    if (silent && silenceStart === null) {
      silenceStart = i;
    } else if (!silent && silenceStart !== null) {
      const durMs = (i - silenceStart) / sampleRate * 1000;
      if (durMs >= gateMs_) {
        gaps.push({ startSample: silenceStart, endSample: i, durationMs: durMs });
      }
      silenceStart = null;
    }
  }

  // Handle trailing silence
  if (silenceStart !== null) {
    const durMs = (samples.length - silenceStart) / sampleRate * 1000;
    if (durMs >= gateMs_) {
      gaps.push({ startSample: silenceStart, endSample: samples.length, durationMs: durMs });
    }
  }

  return gaps;
}

// ─── WAV reassembly ───────────────────────────────────────────────────────────

function writeWavHeader(buf: Buffer, sampleRate: number, numSamples: number): void {
  const dataSize = numSamples * 2;
  buf.write("RIFF", 0);  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write("WAVE", 8);  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(1,  22);  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28);  buf.writeUInt16LE(2, 32);
  buf.writeUInt16LE(16, 34);  buf.write("data", 36);
  buf.writeUInt32LE(dataSize, 40);
}

// ─── Public API ───────────────────────────────────────────────────────────────

/** Cached breath sample per sample rate. */
const breathCache = new Map<number, Float32Array>();

async function getBreathSamples(sampleRate: number, synthUrl: string): Promise<Float32Array> {
  if (breathCache.has(sampleRate)) return breathCache.get(sampleRate)!;
  const wav = await synthesiseBreath(sampleRate, BREATH_DURATION_MS, synthUrl);
  const { samples } = parseWav(wav);
  breathCache.set(sampleRate, samples);
  return samples;
}

export interface BreathInjectorOptions {
  /** Minimum silence gap to inject breath into (ms). Default: 80. */
  silenceGateMs?: number;
  /** Max number of breath injections per buffer. Default: 5. */
  maxInjections?: number;
  /** Whether to inject at the VERY start of the buffer (first phrase). Default: false. */
  injectAtStart?: boolean;
}

/**
 * Inject synthetic breath sounds into silence gaps in a WAV buffer.
 *
 * The breath is placed at the START of each detected silence gap,
 * filling dead air with natural intake texture.
 *
 * @param wav        Input WAV buffer (espeak output, 16-bit mono)
 * @param sampleRate Sample rate of the WAV (usually 22050)
 * @param opts       Injection options
 * @returns          New WAV buffer with breaths spliced in
 */
export async function injectBreaths(
  wav:        Buffer,
  sampleRate: number,
  opts:       BreathInjectorOptions = {},
  synthUrl:   string = "http://localhost:5050",
): Promise<Buffer> {
  const log = getLogger();
  const {
    silenceGateMs   = SILENCE_GATE_MS,
    maxInjections   = 5,
    injectAtStart   = false,
  } = opts;

  const { samples } = parseWav(wav);
  const breath      = await getBreathSamples(sampleRate, synthUrl);
  const gaps        = findSilenceGaps(samples, sampleRate, silenceGateMs);

  // Filter: skip leading silence (first gap if at start) unless injectAtStart
  const candidates = injectAtStart ? gaps : gaps.filter((g, i) => i > 0 || g.startSample > sampleRate * 0.1);

  // Limit injections
  const toInject = candidates.slice(0, maxInjections);

  if (toInject.length === 0) {
    log.debug("BreathInjector", "no suitable silence gaps found — returning unchanged");
    return wav;
  }

  log.info("BreathInjector", "injecting breaths", {
    gaps:    gaps.length,
    injecting: toInject.length,
    gapDurations: toInject.map(g => `${g.durationMs.toFixed(0)}ms`).join(", "),
  });

  // Build new sample array by splicing breath at the start of each gap
  // We insert AT the gap start (replacing first BREATH_DURATION_MS of silence)
  // If the gap is shorter than BREATH_DURATION_MS, we skip.
  const breathLen   = breath.length;
  const regions: Float32Array[] = [];
  let cursor = 0;

  for (const gap of toInject) {
    if (gap.durationMs < BREATH_DURATION_MS) continue;  // gap too short to fit breath
    if (gap.startSample > cursor) {
      regions.push(samples.subarray(cursor, gap.startSample));
    }
    // Place breath at start of gap
    regions.push(breath);
    // Keep remaining silence (gap minus breath duration)
    const remainStart = gap.startSample + breathLen;
    if (remainStart < gap.endSample) {
      regions.push(samples.subarray(remainStart, gap.endSample));
    }
    cursor = gap.endSample;
  }

  // Append remaining audio after last injection
  if (cursor < samples.length) {
    regions.push(samples.subarray(cursor));
  }

  // Assemble new sample array
  const totalSamples = regions.reduce((s, r) => s + r.length, 0);
  const out          = new Float32Array(totalSamples);
  let offset = 0;
  for (const r of regions) { out.set(r, offset); offset += r.length; }

  // Encode back to 16-bit WAV
  const outBuf = Buffer.alloc(44 + totalSamples * 2);
  writeWavHeader(outBuf, sampleRate, totalSamples);
  for (let i = 0; i < totalSamples; i++) {
    const s = Math.max(-1, Math.min(1, out[i]!));
    outBuf.writeInt16LE(Math.round(s * 32767), 44 + i * 2);
  }

  return outBuf;
}

// ─── AudioProcessor wrapper ───────────────────────────────────────────────────

/**
 * Wraps any AudioProcessor, injecting breath sounds before passing to inner.
 *
 * Usage:
 *   new BreathProcessor(new SmoothingProcessor(new RVCProcessor(url)))
 */
export class BreathProcessor implements AudioProcessor {
  constructor(
    private readonly inner:    AudioProcessor,
    private readonly opts:     BreathInjectorOptions = {},
    private readonly synthUrl: string = "http://localhost:5050",
  ) {}

  async process(wav: Buffer): Promise<Buffer> {
    const { sampleRate } = parseWav(wav);
    const enriched       = await injectBreaths(wav, sampleRate, this.opts, this.synthUrl);
    return this.inner.process(enriched);
  }
}
