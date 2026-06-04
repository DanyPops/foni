import { rms, peak, parseWav } from "./analysis/audio-utils.ts";

interface AudioProcessor {
  process(input: Buffer): Promise<Buffer>;
}

const log = {
  debug: (..._args: unknown[]) => {},
  info:  (..._args: unknown[]) => {},
  warn:  (..._args: unknown[]) => {},
};


// ─── Constants ────────────────────────────────────────────────────────────────

/** Default RVC HTTP request timeout. CPU inference can be slow. */
export const DEFAULT_RVC_TIMEOUT_MS = 60_000;

// ─── WAV pad helper (replaces ffmpeg adelay+apad) ────────────────────────────

/** Encode f32 samples as 16-bit PCM WAV (mono). */
function encodeWav(samples: Float32Array, sampleRate: number): Buffer {
  const n   = samples.length;
  const buf = Buffer.alloc(44 + n * 2);
  buf.write("RIFF", 0); buf.writeUInt32LE(36 + n * 2, 4);
  buf.write("WAVE", 8); buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(sampleRate, 24); buf.writeUInt32LE(sampleRate * 2, 28);
  buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34);
  buf.write("data", 36); buf.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) {
    buf.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(samples[i] * 32767))), 44 + i * 2);
  }
  return buf;
}

/** Prepend and append padSecs of silence to a WAV buffer. */
function padWavSilence(input: Buffer, padSecs: number): Buffer {
  if (padSecs <= 0) return input;
  const { samples, sampleRate } = parseWav(input);
  const padN   = Math.round(sampleRate * padSecs);
  const padded = new Float32Array(padN + samples.length + padN);
  padded.set(samples, padN);
  return encodeWav(padded, sampleRate);
}

// ─── SmoothingProcessor ───────────────────────────────────────────────────────

export interface SmoothingOptions {
  // ── Edge handling ───────────────────────────────────────────────────────────

  /**
   * Silence added before AND after the audio before inner.process().
   * Gives RVC windowed inference edge context — prevents swallowed starts/ends.
   * Seconds. Default: 0.3
   */
  padSecs: number;

  /**
   * Fade-in and fade-out duration on the final output.
   * Removes the hard click at the RVC-converted audio edges.
   * Seconds. Default: 0.04
   */
  fadeSecs: number;

  // ── Mud removal ─────────────────────────────────────────────────────────────

  /**
   * High-pass filter cutoff in Hz. Removes sub-bass rumble that espeak
   * and RVC can introduce — keeps the voice clean below the fundamentals.
   * 0 = off. Default: 80
   */
  highpassFreq: number;

  // ── Corrective EQ (remove before adding) ───────────────────────────────────

  /**
   * Frequency in Hz to cut for "boxy" hollow resonance.
   * 800–1000 Hz is where espeak's formant synthesis sounds hollow.
   * Default: 900
   */
  deBoxFreq: number;

  /** Gain in dB at deBoxFreq. Negative = cut. Default: -2 */
  deBoxDb: number;

  /** Bandwidth in octaves for the deBox EQ band. Default: 1.5 */
  deBoxBandwidthOctaves: number;

  /**
   * Frequency in Hz to cut for harsh metallic character.
   * 3000–4000 Hz is where espeak's formant synthesis sounds metallic.
   * Default: 3500
   */
  deHarshFreq: number;

  /** Gain in dB at deHarshFreq. Negative = cut. Default: -2 */
  deHarshDb: number;

  /** Bandwidth in octaves for the deHarsh EQ band. Default: 2 */
  deHarshBandwidthOctaves: number;

  // ── Dynamics ────────────────────────────────────────────────────────────────

  /**
   * Compression ratio (N:1). 1 = no compression. 3 = gentle, natural.
   * Evens out the flat, mechanical dynamics of synthetic speech.
   * Default: 3
   */
  compressionRatio: number;

  /**
   * Compressor attack time in ms. How fast it responds to loud sounds.
   * Slow attack (15–30ms) lets word transients through naturally.
   * Default: 20
   */
  compressionAttackMs: number;

  /**
   * Compressor release time in ms. How fast it recovers between words.
   * 100–200ms gives natural "breathing" between phrases.
   * Default: 150
   */
  compressionReleaseMs: number;

  /**
   * Compression threshold in dB. Signal above this gets compressed.
   * Default: -18 (≈ 0.125 linear)
   */
  compressionThresholdDb: number;

  /**
   * Makeup gain in dB after compression to restore perceived loudness.
   * Default: 2
   */
  compressionMakeupDb: number;

  // ── Creative EQ (add after dynamics) ───────────────────────────────────────

  /**
   * Warmth: low-shelf boost gain in dB at warmthFreq.
   * 150–300 Hz is where voice body and "chest resonance" lives.
   * 0 = off. Default: 2.5
   */
  warmthBoostDb: number;

  /** Centre frequency in Hz for the warmth low-shelf. Default: 200 */
  warmthFreq: number;

  /**
   * Air: high-shelf boost gain in dB above airFreq.
   * Above 8 kHz is where sparkle, presence, and "breath" live.
   * 0 = off. Default: 1.0
   */
  airBoostDb: number;

  /** Frequency in Hz above which the air shelf kicks in. Default: 8000 */
  airFreq: number;

  // ── Harmonic saturation (THE warmth key) ────────────────────────────────────

  /**
   * Harmonic exciter drive (aexciter `drive`). Adds synthetic odd harmonics
   * above saturationFreq — the same effect as tape/tube saturation.
   * This is what makes synthetic voices sound warm and organic.
   * Range: 0.1–10. 0 = off. Default: 3.0
   */
  saturationDrive: number;

  /**
   * Amount of exciter signal mixed in (aexciter `amount`). 0–64.
   * Lower = subtler blend with dry signal. Default: 1.0
   */
  saturationAmount: number;

  /**
   * Lowest frequency in Hz the exciter will process.
   * Range: 2000–12000. Default: 6000
   */
  saturationFreq: number;

  // ── Spatial depth ───────────────────────────────────────────────────────────

  /**
   * Phaser decay (depth). Creates subtle phase movement that adds
   * dimension — makes the voice feel less "flat" and more 3D.
   * Range: 0–0.99. 0 = off. Default: 0.2
   */
  phaserDepth: number;

  /**
   * Room reverb delay in ms. Short (10–20ms) = small room.
   * 0 = reverb off. Default: 15
   */
  reverbMs: number;

  /** Reverb decay factor 0–1. 0.05–0.15 = subtle warmth. Default: 0.08 */
  reverbDecay: number;

  /** Reverb input gain 0–1 (aecho in_gain). Default: 0.8 */
  reverbInputGain: number;

  /** Reverb output/wet gain 0–1 (aecho out_gain). Default: 0.88 */
  reverbOutputGain: number;

  // ── Final ───────────────────────────────────────────────────────────────────

  /**
   * Apply EBU R128 loudnorm as the final stage for consistent volume.
   * Default: true
   */
  // ── De-robotisation ─────────────────────────────────────────────────────────

  /** Breathiness: pink noise floor in dB (negative). 0 = off. Typical: −50 to −35. */
  breathinessDb: number;
  /** Spectral tilt low-shelf: +dB at 100Hz to restore chest resonance. 0 = off. */
  tiltLowDb: number;
  /** Spectral tilt high-shelf: dB at 8kHz (negative = cut). 0 = off. */
  tiltHighDb: number;
  /** Presence boost: +dB at 2.5kHz voice clarity band. 0 = off. */
  presenceDb: number;
  /** De-esser: reduction in dB at 7kHz (positive = cut). 0 = off. */
  deEssDb: number;
  /** Vibrato modulation frequency in Hz. Breaks mechanical espeak pitch. 0 = off. */
  vibratoFreq: number;
  /** Vibrato modulation depth 0–1. 0.003 = ±0.3% pitch variation. 0 = off. */
  vibratoDepth: number;

  // ── Output normalisation & dynamics ceiling ──────────────────────────────

  /**
   * loudnorm target integrated loudness in LUFS (negative number).
   * −14 LUFS ≈ −12 to −13 dBFS RMS.
   * 0 = use bare loudnorm (old behaviour). Off when normalize=false.
   */
  rmsTargetLufs: number;

  /**
   * Hard limiter ceiling in dBFS (negative number, e.g. −1).
   * Caps peaks without changing average level — reduces crest factor.
   * 0 = off. Applied after loudnorm.
   */
  limiterDb: number;

  /**
   * Silence-trim threshold in dBFS for edge silence removal.
   * Strips leading/trailing silence below this level at the audio edges. 0 = off.
   */
  silenceTrimDb: number;

  normalize: boolean;
}

export const DEFAULT_SMOOTHING: SmoothingOptions = {
  // Edge handling
  padSecs:  0.3,
  fadeSecs: 0.04,

  // Mud removal
  highpassFreq: 80,

  // Corrective EQ
  deBoxFreq:             900,
  deBoxDb:               0,
  deBoxBandwidthOctaves: 1.5,
  deHarshFreq:             3500,
  deHarshDb:               -2,
  deHarshBandwidthOctaves: 2,

  // Dynamics — tighter compression to reduce crest factor gap
  compressionRatio:       4,
  compressionAttackMs:    10,
  compressionReleaseMs:   80,
  compressionThresholdDb: -12,
  compressionMakeupDb:    5,

  // Creative EQ
  warmthBoostDb: 0,
  warmthFreq:    200,
  airBoostDb:    0,
  airFreq:       8000,

  // Harmonic saturation — aexciter valid range is 2000–12000 Hz; disabled because
  // 2000 Hz saturates the presence band and degrades spectral tilt.
  saturationDrive:  0,
  saturationAmount: 0,
  saturationFreq:   2000,

  // Spatial depth
  phaserDepth:      0.08,
  reverbMs:         8,
  reverbDecay:      0.04,
  reverbInputGain:  0.8,
  reverbOutputGain: 0.88,

  // De-robotisation — tuned to close centroid and RMS gap
  breathinessDb: 0,
  tiltLowDb:     10,
  tiltHighDb:    -8,
  presenceDb:    0,
  deEssDb:       4,

  // Pitch micro-variation
  vibratoFreq:  6,
  vibratoDepth: 0.003,

  // Output normalisation
  rmsTargetLufs: -8,
  limiterDb:     -1,
  silenceTrimDb: -40,

  normalize: true,
};

/**
 * SmoothingProcessor — full DSP chain via foni-synth Rust server.
 *
 * Signal chain:
 *   Pre:  pad silence (pure WAV) → RVC voice conversion
 *   Post: Rust DSP via POST /process (no ffmpeg)
 *         silence trim → fade → highpass → tilt → de-ess → vibrato →
 *         corrective EQ → compression → warmth → air → reverb → loudnorm → clip
 */
// ─── Auto-label: describe a SmoothingOptions diff against DEFAULT_SMOOTHING ───────────
//
// Generates a human-readable string from changed fields only.
// Companion fields are absorbed into their primary (e.g. reverbDecay → reverbMs).
// Empty diff → 'baseline (DEFAULT_SMOOTHING)'.
//
// Usage in tuning.e2e.test.ts:
//   const CONFIGS = [
//     { name: '1. baseline', opts: {} },
//     { name: '2. exciter', opts: { saturationDrive: 2.5 } },
//   ];
//   // label auto-generated: 'exciter drive=2.5 @ 5kHz'

/** Fields that are described via another primary field — skip in diff output. */
const ABSORBED_FIELDS = new Set<keyof SmoothingOptions>([
  "saturationAmount", "saturationFreq",       // → saturationDrive
  "reverbDecay", "reverbInputGain", "reverbOutputGain", // → reverbMs
  "vibratoDepth",                                        // → vibratoFreq
  "deHarshFreq", "deHarshBandwidthOctaves",   // → deHarshDb
  "deBoxFreq", "deBoxBandwidthOctaves",        // → deBoxDb
  "warmthFreq",                                // → warmthBoostDb
  "airFreq",                                   // → airBoostDb
  "compressionReleaseMs", "compressionThresholdDb", // → compressionRatio
]);

function describeField(
  key: keyof SmoothingOptions,
  val: number | boolean,
  opts: Partial<SmoothingOptions>,
  base: SmoothingOptions,
): string {
  const hz = (f: number) => f >= 1000 ? `${(f / 1000).toFixed(f % 1000 === 0 ? 0 : 1)}kHz` : `${f}Hz`;
  const db = (v: number) => `${v > 0 ? '+' : ''}${v}dB`;

  switch (key) {
    case "saturationDrive": {
      const freq = hz(opts.saturationFreq ?? base.saturationFreq);
      return `exciter drive=${val} @ ${freq}`;
    }
    case "deHarshDb": {
      const freq = hz(opts.deHarshFreq ?? base.deHarshFreq);
      return `${Number(val) > 0 ? 'presence' : 'de-harsh'} ${db(Number(val))} @ ${freq}`;
    }
    case "deBoxDb": {
      const freq = hz(opts.deBoxFreq ?? base.deBoxFreq);
      return `de-box ${db(Number(val))} @ ${freq}`;
    }
    case "warmthBoostDb": {
      const freq = hz(opts.warmthFreq ?? base.warmthFreq);
      return `warmth ${db(Number(val))} @ ${freq}`;
    }
    case "airBoostDb": {
      const freq = hz(opts.airFreq ?? base.airFreq);
      return `air ${db(Number(val))} @ ${freq}`;
    }
    case "reverbMs": {
      const decay  = ((opts.reverbDecay ?? base.reverbDecay) * 100).toFixed(0);
      return `reverb ${val}ms / ${decay}% decay`;
    }
    case "breathinessDb":      return `breathiness ${val}dB`;
    case "tiltLowDb":         return `tilt +${val}dB@100Hz`;
    case "tiltHighDb":         return `tilt ${val}dB@8kHz`;
    case "presenceDb":         return `presence ${Number(val) > 0 ? "+" : ""}${val}dB@2.5kHz`;
    case "deEssDb":            return `de-ess −${val}dB@7kHz`;
    case "phaserDepth":        return `phaser depth=${val}`;
    case "compressionRatio":   return `compression ${val}:1`;
    case "compressionAttackMs": return `attack ${val}ms`;
    case "compressionMakeupDb": return Number(val) !== 0 ? `makeup ${db(Number(val))}` : "";
    case "highpassFreq":       return `highpass ${hz(Number(val))}`;
    case "fadeSecs":           return `fade ${val}s`;
    case "padSecs":            return `pad ${val}s`;
    case "rmsTargetLufs":     return `loudnorm I=${val}LUFS`;
    case "limiterDb":          return `limit ${val}dBFS`;
    case "silenceTrimDb":      return val ? `silence-trim ${val}dB` : "no-silence-trim";
    case "normalize":          return val ? "loudnorm" : "no-loudnorm";
    default:                   return `${key}=${val}`;
  }
}

/**
 * Generate a human-readable description of opts relative to DEFAULT_SMOOTHING.
 * Each changed field becomes a clause; absorbed companion fields are skipped.
 *
 * @example
 * describeSmoothingDiff({}) // 'baseline (DEFAULT_SMOOTHING)'
 * describeSmoothingDiff({ saturationDrive: 2.5, reverbMs: 20, reverbDecay: 0.08 })
 * // 'exciter drive=2.5 @ 5kHz, reverb 20ms / 8% decay'
 */
export function describeSmoothingDiff(
  opts: Partial<SmoothingOptions>,
  base: SmoothingOptions = DEFAULT_SMOOTHING,
): string {
  const clauses: string[] = [];

  for (const [k, val] of Object.entries(opts) as [keyof SmoothingOptions, number | boolean][]) {
    if (ABSORBED_FIELDS.has(k)) continue;
    if (val === (base as unknown as Record<string, unknown>)[k]) continue; // unchanged
    const clause = describeField(k, val, opts, base);
    if (clause) clauses.push(clause);
  }

  return clauses.length === 0 ? "baseline (DEFAULT_SMOOTHING)" : clauses.join(", ");
}

export class SmoothingProcessor implements AudioProcessor {
  private readonly opts: SmoothingOptions;

  /**
   * @param inner      Underlying processor (typically RVCProcessor).
   * @param opts       DSP parameter overrides against DEFAULT_SMOOTHING.
   * @param synthUrl   Base URL of foni-synth server — POST /process endpoint.
   *                   Defaults to same host:port as the RVC server (5050).
   */
  constructor(
    private readonly inner: AudioProcessor,
    opts: Partial<SmoothingOptions> = {},
    private readonly synthUrl: string = "http://localhost:5050",
  ) {
    this.opts = { ...DEFAULT_SMOOTHING, ...opts };
  }

  // buildPreFilter / buildPostFilter / ffmpegFilter removed — replaced by
  // padWavSilence() + POST /process (Rust DSP chain in foni-synth).

  // DEAD CODE MARKER — kept for grepping during migration review:

  async process(input: Buffer): Promise<Buffer> {
    // logging removed

    // 1. Pad silence — gives RVC edge context (pure WAV manipulation, no ffmpeg)
    const padded = padWavSilence(input, this.opts.padSecs);
    log.debug("SmoothingProcessor", "padded", { inLen: input.length, outLen: padded.length });

    // 2. RVC voice conversion
    const rvcOut = await this.inner.process(padded);
    log.debug("SmoothingProcessor", "post-RVC", { len: rvcOut.length });

    // 3. DSP chain via foni-synth /process (Rust — no ffmpeg)
    try {
      const resp = await fetch(`${this.synthUrl}/process`, {
        method:  "POST",
        headers: { "Content-Type": "application/json" },
        body:    JSON.stringify({ audio_data: rvcOut.toString("base64"), opts: this.opts }),
        signal:  AbortSignal.timeout(30_000),
      });
      if (!resp.ok) {
        const msg = `foni-synth /process failed (HTTP ${resp.status})`;
        if (process.env.FONI_REQUIRE_DSP === "1") throw new Error(msg);
        log.warn("SmoothingProcessor", `${msg} — returning RVC output`);
        return rvcOut;
      }
      const { audio_data } = await resp.json() as { audio_data: string };
      const final = Buffer.from(audio_data, "base64");
      log.debug("SmoothingProcessor", "post-DSP", { len: final.length });
      return final;
    } catch (e: any) {
      const msg = `/process unreachable: ${e?.message}`;
      if (process.env.FONI_REQUIRE_DSP === "1") throw new Error(msg);
      log.warn("SmoothingProcessor", `${msg} — returning RVC output`);
      return rvcOut;
    }
  }
}

// ─── IdentityProcessor ────────────────────────────────────────────────────────

export class IdentityProcessor implements AudioProcessor {
  async process(input: Buffer): Promise<Buffer> {
    return input;
  }
}

// ─── RVCProcessor ─────────────────────────────────────────────────────────────

export class RVCProcessor implements AudioProcessor {
  constructor(
    private readonly url: string,
    private readonly timeoutMs = DEFAULT_RVC_TIMEOUT_MS,
  ) {}

  async process(input: Buffer): Promise<Buffer> {
    // logging removed
    const start = Date.now();
    try {
      const resp = await fetch(`${this.url}/convert`, {
        method:  "POST",
        headers: { "Content-Type": "application/json" },
        body:    JSON.stringify({ audio_data: input.toString("base64") }),
        signal:  AbortSignal.timeout(this.timeoutMs),
      });
      const ms = Date.now() - start;
      if (!resp.ok) {
        log.warn("RVCProcessor", "HTTP error — returning identity",
          { status: resp.status, ms, url: this.url });
        return input;
      }
      const out = Buffer.from(await resp.arrayBuffer());
      log.info("RVCProcessor", "convert OK",
        { ms, inBytes: input.length, outBytes: out.length });
      return out;
    } catch (err) {
      const ms = Date.now() - start;
      log.warn("RVCProcessor", "request failed — returning identity",
        { ms, error: String(err), url: this.url });
      return input;
    }
  }
}
