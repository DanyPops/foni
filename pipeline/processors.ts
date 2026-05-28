import { spawnSync } from "node:child_process";
import type { AudioProcessor } from "./interfaces.ts";

// ─── Constants ──────────────────────────────────────────────────────────────────

/** Maximum WAV payload ffmpeg will read/write via stdio pipe. */
const FFMPEG_MAX_BUFFER_BYTES = 50 * 1024 * 1024;

/** Default RVC HTTP request timeout. CPU inference can be slow. */
export const DEFAULT_RVC_TIMEOUT_MS = 60_000;

// ─── ffmpeg helper ──────────────────────────────────────────────────────────────

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

export class IdentityProcessor implements AudioProcessor {
  async process(input: Buffer): Promise<Buffer> {
    return input;
  }
}

// ─── SmoothingProcessor ──────────────────────────────────────────────────────────

export interface SmoothingOptions {
  /**
   * Silence added before AND after the audio before inner.process().
   * Gives RVC windowed inference edge context — prevents swallowed starts/ends.
   * Seconds. Default: 0.3
   */
  padSecs: number;

  /**
   * Fade-in and fade-out duration applied to the final output.
   * Removes the hard click at the start and end of RVC-converted audio.
   * Seconds. Default: 0.04
   */
  fadeSecs: number;

  /**
   * Room reverb delay in milliseconds. 0 = reverb off.
   * Short (10–20ms) = small room. Long (40–100ms) = hall.
   * Default: 15
   */
  reverbMs: number;

  /**
   * Reverb decay factor 0–1. Higher = longer echo tail.
   * 0.05–0.15 = subtle warmth. 0.4+ = cave/cathedral.
   * Default: 0.08
   */
  reverbDecay: number;

  /**
   * Reverb input gain (ffmpeg aecho in_gain). Controls how much of the
   * dry signal enters the reverb. 0–1, default: 0.8
   */
  reverbInputGain: number;

  /**
   * Reverb output gain (ffmpeg aecho out_gain). Controls the wet level.
   * 0–1, default: 0.88
   */
  reverbOutputGain: number;

  /**
   * EQ centre frequency in Hz to cut or boost.
   * 3000–4000 Hz is where espeak’s harsh formant character lives.
   * Default: 3500
   */
  eqFreq: number;

  /**
   * EQ gain in dB at eqFreq. Negative = cut (soften), positive = boost.
   * Default: -2
   */
  eqGain: number;

  /**
   * EQ filter bandwidth in octaves.
   * 1 = narrow notch. 2 = broad shelf. Default: 2
   */
  eqBandwidthOctaves: number;

  /**
   * Apply loudnorm to keep output volume consistent regardless of reverb/EQ.
   * Default: true
   */
  normalize: boolean;
}

export const DEFAULT_SMOOTHING: SmoothingOptions = {
  padSecs:          0.3,
  fadeSecs:         0.04,
  reverbMs:         15,
  reverbDecay:      0.08,
  reverbInputGain:  0.8,
  reverbOutputGain: 0.88,
  eqFreq:           3500,
  eqGain:           -2,
  eqBandwidthOctaves: 2,
  normalize:        true,
};

/**
 * SmoothingProcessor — wraps any AudioProcessor with ffmpeg pre/post passes.
 *
 * Pre-inner:  pads silence so the inner processor (e.g. RVC) has edge context.
 * Post-inner: fade in/out + optional reverb + optional EQ + loudnorm.
 *
 * All parameters are exposed as named SmoothingOptions — no raw filter strings.
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
    const ms = Math.round(padSecs * 1000);
    return `adelay=${ms}|${ms},apad=pad_dur=${padSecs}`;
  }

  private buildPostFilter(): string {
    const { fadeSecs, reverbMs, reverbDecay, reverbInputGain, reverbOutputGain, eqFreq, eqGain, eqBandwidthOctaves, normalize } = this.opts;
    const parts: string[] = [];

    // Fade in
    parts.push(`afade=t=in:d=${fadeSecs}`);

    // Fade out via reverse trick (no need to know duration)
    parts.push(`areverse,afade=t=in:d=${fadeSecs},areverse`);

    // Optional reverb
    if (reverbMs > 0 && reverbDecay > 0) {
      parts.push(`aecho=${reverbInputGain}:${reverbOutputGain}:${reverbMs}:${reverbDecay}`);
    }

    // Optional EQ
    if (eqGain !== 0) {
      parts.push(`equalizer=f=${eqFreq}:width_type=o:width=${eqBandwidthOctaves}:g=${eqGain}`);
    }

    // Optional loudnorm
    if (normalize) parts.push("loudnorm");

    return parts.join(",");
  }

  async process(input: Buffer): Promise<Buffer> {
    const padded = ffmpegFilter(input, this.buildPreFilter());
    const rvcOut = await this.inner.process(padded);
    return ffmpegFilter(rvcOut, this.buildPostFilter());
  }
}

export class RVCProcessor implements AudioProcessor {
  constructor(private readonly url: string, private readonly timeoutMs = DEFAULT_RVC_TIMEOUT_MS) {}

  async process(input: Buffer): Promise<Buffer> {
    try {
      const resp = await fetch(`${this.url}/convert`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ audio_data: input.toString("base64") }),
        signal: AbortSignal.timeout(this.timeoutMs),
      });
      if (!resp.ok) return input;
      return Buffer.from(await resp.arrayBuffer());
    } catch {
      return input;
    }
  }
}
