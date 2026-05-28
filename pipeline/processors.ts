import { spawnSync } from "node:child_process";
import type { AudioProcessor } from "./interfaces.ts";

// ─── Constants ────────────────────────────────────────────────────────────────

/** Maximum WAV payload ffmpeg will read/write via stdio pipe. */
const FFMPEG_MAX_BUFFER_BYTES = 50 * 1024 * 1024;

/** Default RVC HTTP request timeout. CPU inference can be slow. */
export const DEFAULT_RVC_TIMEOUT_MS = 60_000;

// ─── ffmpeg helper ────────────────────────────────────────────────────────────

/**
 * Run an ffmpeg audio filter chain on a WAV buffer via stdio pipes.
 * Returns the original buffer unchanged if ffmpeg is unavailable or errors.
 */
function ffmpegFilter(input: Buffer, af: string): Buffer {
  const result = spawnSync(
    "ffmpeg",
    ["-hide_banner", "-loglevel", "error",
     "-i", "pipe:0",
     "-af", af,
     "-f", "wav", "pipe:1"],
    { input, maxBuffer: FFMPEG_MAX_BUFFER_BYTES },
  );
  if (result.error || result.status !== 0 || !result.stdout?.length) return input;
  return result.stdout as Buffer;
}

/** Convert dB to the linear amplitude scale ffmpeg acompressor expects. */
function dbToLinear(db: number): number {
  return Math.pow(10, db / 20);
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
  deBoxDb:               -2,
  deBoxBandwidthOctaves: 1.5,
  deHarshFreq:             3500,
  deHarshDb:               -2,
  deHarshBandwidthOctaves: 2,

  // Dynamics
  compressionRatio:       3,
  compressionAttackMs:    20,
  compressionReleaseMs:   150,
  compressionThresholdDb: -18,
  compressionMakeupDb:    2,

  // Creative EQ
  warmthBoostDb: 2.5,
  warmthFreq:    200,
  airBoostDb:    1.0,
  airFreq:       8000,

  // Harmonic saturation
  saturationDrive:  3.0,
  saturationAmount: 1.0,
  saturationFreq:   6000,

  // Spatial depth
  phaserDepth:      0.2,
  reverbMs:         15,
  reverbDecay:      0.08,
  reverbInputGain:  0.8,
  reverbOutputGain: 0.88,

  // Final
  normalize: true,
};

/**
 * SmoothingProcessor — wraps any AudioProcessor with ffmpeg pre/post passes.
 *
 * Signal chain (professional audio order):
 *   Pre:  pad silence → RVC has edge context
 *   Post: fade → highpass → corrective EQ → compression →
 *         warmth EQ → air EQ → harmonic saturation → phaser →
 *         reverb → loudnorm
 *
 * All parameters exposed as named SmoothingOptions — no raw filter strings.
 * Falls back transparently if ffmpeg is not installed.
 */
export class SmoothingProcessor implements AudioProcessor {
  private readonly opts: SmoothingOptions;

  constructor(
    private readonly inner: AudioProcessor,
    opts: Partial<SmoothingOptions> = {},
  ) {
    this.opts = { ...DEFAULT_SMOOTHING, ...opts };
  }

  private buildPreFilter(): string {
    const { padSecs } = this.opts;
    if (padSecs <= 0) return "anull";  // identity — no padding
    const ms = Math.round(padSecs * 1000);
    return `adelay=${ms}|${ms},apad=pad_dur=${padSecs}`;
  }

  private buildPostFilter(): string {
    const {
      fadeSecs,
      highpassFreq,
      deBoxFreq, deBoxDb, deBoxBandwidthOctaves,
      deHarshFreq, deHarshDb, deHarshBandwidthOctaves,
      compressionRatio, compressionAttackMs, compressionReleaseMs,
      compressionThresholdDb, compressionMakeupDb,
      warmthBoostDb, warmthFreq,
      airBoostDb, airFreq,
      saturationDrive, saturationAmount, saturationFreq,
      phaserDepth,
      reverbMs, reverbDecay, reverbInputGain, reverbOutputGain,
      normalize,
    } = this.opts;

    const parts: string[] = [];

    // 1. Fade in / out — only if non-zero (afade=d:0 silences the signal)
    if (fadeSecs > 0) {
      parts.push(`afade=t=in:d=${fadeSecs}`);
      parts.push(`areverse,afade=t=in:d=${fadeSecs},areverse`);
    }

    // 2. Mud removal — strip sub-bass before any boosting
    if (highpassFreq > 0) {
      parts.push(`highpass=f=${highpassFreq}`);
    }

    // 3. Corrective EQ — cut problems before adding character
    if (deBoxDb !== 0) {
      parts.push(
        `equalizer=f=${deBoxFreq}:width_type=o:width=${deBoxBandwidthOctaves}:g=${deBoxDb}`,
      );
    }
    if (deHarshDb !== 0) {
      parts.push(
        `equalizer=f=${deHarshFreq}:width_type=o:width=${deHarshBandwidthOctaves}:g=${deHarshDb}`,
      );
    }

    // 4. Compression — even out dynamics before boosting anything
    if (compressionRatio > 1) {
      const threshold = dbToLinear(compressionThresholdDb);
      const makeup    = dbToLinear(compressionMakeupDb);
      parts.push(
        `acompressor=threshold=${threshold.toFixed(5)}` +
        `:ratio=${compressionRatio}` +
        `:attack=${compressionAttackMs}` +
        `:release=${compressionReleaseMs}` +
        `:makeup=${makeup.toFixed(5)}`,
      );
    }

    // 5. Warmth — low shelf boost for body and chest resonance
    if (warmthBoostDb !== 0) {
      parts.push(`lowshelf=g=${warmthBoostDb}:f=${warmthFreq}:width_type=s:width=0.5`);
    }

    // 6. Air — high shelf boost for sparkle and presence above 8kHz
    if (airBoostDb !== 0) {
      parts.push(`highshelf=g=${airBoostDb}:f=${airFreq}:width_type=s:width=0.5`);
    }

    // 7. Harmonic saturation — adds synthetic odd harmonics (tape/tube warmth)
    //    This is the single biggest upgrade over plain EQ.
    if (saturationDrive > 0 && saturationAmount > 0) {
      parts.push(
        `aexciter=freq=${saturationFreq}` +
        `:drive=${saturationDrive}` +
        `:amount=${saturationAmount}` +
        `:level_in=1:level_out=1:blend=0`,
      );
    }

    // 8. Phase depth — subtle movement makes voice feel less flat
    if (phaserDepth > 0) {
      parts.push(
        `aphaser=in_gain=0.4:out_gain=0.74:delay=3:decay=${phaserDepth}:speed=0.5`,
      );
    }

    // 9. Room reverb — spatial placement last before normalize
    if (reverbMs > 0 && reverbDecay > 0) {
      parts.push(
        `aecho=${reverbInputGain}:${reverbOutputGain}:${reverbMs}:${reverbDecay}`,
      );
    }

    // 10. Loudnorm — always the final stage
    if (normalize) parts.push("loudnorm");

    return parts.join(",");
  }

  async process(input: Buffer): Promise<Buffer> {
    const padded = ffmpegFilter(input, this.buildPreFilter());
    const rvcOut = await this.inner.process(padded);
    return ffmpegFilter(rvcOut, this.buildPostFilter());
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
    try {
      const resp = await fetch(`${this.url}/convert`, {
        method:  "POST",
        headers: { "Content-Type": "application/json" },
        body:    JSON.stringify({ audio_data: input.toString("base64") }),
        signal:  AbortSignal.timeout(this.timeoutMs),
      });
      if (!resp.ok) return input;
      return Buffer.from(await resp.arrayBuffer());
    } catch {
      return input;
    }
  }
}
