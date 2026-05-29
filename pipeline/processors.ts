import { spawnSync }   from "node:child_process";
import type { AudioProcessor } from "./interfaces.ts";
import { getLogger }   from "../core/logger.ts";
import { rms, parseWav } from "./audio-utils.ts";

// ─── Constants ────────────────────────────────────────────────────────────────

/** Maximum WAV payload ffmpeg will read/write via stdio pipe. */
const FFMPEG_MAX_BUFFER_BYTES = 50 * 1024 * 1024;

/** Default RVC HTTP request timeout. CPU inference can be slow. */
export const DEFAULT_RVC_TIMEOUT_MS = 60_000;

// ─── ffmpeg helper ────────────────────────────────────────────────────────────

/**
 * Run an ffmpeg audio filter chain on a WAV buffer via stdio pipes.
 * Returns the original buffer unchanged if ffmpeg is unavailable or errors,
 * and emits a WARN log so silent fallbacks are never invisible.
 */
function ffmpegFilter(input: Buffer, af: string): Buffer {
  const log = getLogger();
  log.debug("ffmpeg", "apply filter", { len: input.length, af: af.slice(0, 120) });

  const result = spawnSync(
    "ffmpeg",
    ["-hide_banner", "-loglevel", "error",
     "-i", "pipe:0",
     "-af", af,
     "-f", "wav", "pipe:1"],
    { input, maxBuffer: FFMPEG_MAX_BUFFER_BYTES },
  );

  if (result.error || result.status !== 0 || !result.stdout?.length) {
    const stderr = result.stderr?.toString().trim() ?? "";
    log.warn("ffmpeg", "filter failed — returning identity", {
      status: result.status,
      error:  result.error?.message,
      stderr: stderr.slice(0, 200),
      af:     af.slice(0, 120),
    });
    return input;
  }

  log.debug("ffmpeg", "filter OK", { inLen: input.length, outLen: result.stdout.length });
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
   * −14 LUFS ≈ −12 to −13 dBFS RMS, matching STALKER studio recordings.
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
   * Strips leading/trailing silence below this level — closes voiced-ratio gap.
   * −40 = trim everything below −40 dBFS at the edges. 0 = off.
   */
  silenceTrimDb: number;

  normalize: boolean;
}

// Baseline v5: round 5 winner — all-derobot stack.
// Round 1: natural-dry (trust RVC) won.
// Round 2: de-harsh + punch added.
// Round 3: all-three anti-robotic stack won — exciter(1.5@5kHz) + phaser(0.15) + reverb(12ms/6%).
// Round 5: de-robotisation stack won — breathiness + tilt + de-ess + presence + exciter→1.2kHz.
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

  // Dynamics — punchy
  compressionRatio:       2,
  compressionAttackMs:    20,
  compressionReleaseMs:   150,
  compressionThresholdDb: -18,
  compressionMakeupDb:    1,

  // Creative EQ
  warmthBoostDb: 0,
  warmthFreq:    200,
  airBoostDb:    0,
  airFreq:       8000,

  // Harmonic saturation — aexciter valid range: 2000–12000 Hz
  // 2kHz = lowest valid value; warm character without high harshness
  saturationDrive:  1.5,
  saturationAmount: 1.0,
  saturationFreq:   2000,

  // Spatial depth — shorter/subtler than round 3
  phaserDepth:      0.08,
  reverbMs:         8,
  reverbDecay:      0.04,
  reverbInputGain:  0.8,
  reverbOutputGain: 0.88,

  // De-robotisation — round 5 all-derobot stack baked in
  // FON-TSK-51: tilt increased toward STALKER baseline (target 40 dB vs our 22–32 dB)
  // FON-TSK-52: breathiness nudged quieter so silenceTrimDb doesn’t preserve noise
  breathinessDb: -48,
  tiltLowDb:     5,
  tiltHighDb:    -4,
  presenceDb:    1.5,
  deEssDb:       4,

  // Pitch micro-variation — off by default (round 6)
  vibratoFreq:  0,
  vibratoDepth: 0,

  // Output normalisation — FON-TSK-48/49/50
  // Target −14 LUFS ≈ −12 to −13 dBFS RMS (matches STALKER baseline −12.6 dBFS)
  rmsTargetLufs: -14,
  // Hard limiter at −1 dBFS — tames peaks, closes crest-factor gap (FON-TSK-50)
  limiterDb:     -1,
  // Trim edge silence — closes voiced-ratio gap from 28% → ~55% (FON-TSK-49)
  silenceTrimDb: -40,

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
    case "presenceDb":         return `presence ${val > 0 ? "+" : ""}${val}dB@2.5kHz`;
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
    if (val === (base as Record<string, unknown>)[k]) continue; // unchanged
    const clause = describeField(k, val, opts, base);
    if (clause) clauses.push(clause);
  }

  return clauses.length === 0 ? "baseline (DEFAULT_SMOOTHING)" : clauses.join(", ");
}

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
      breathinessDb, tiltLowDb, tiltHighDb, presenceDb, deEssDb,
      vibratoFreq, vibratoDepth,
      normalize, rmsTargetLufs, limiterDb, silenceTrimDb,
    } = this.opts;

    const parts: string[] = [];

    // 0. Edge-silence trim — strip espeak leading/trailing silence before processing.
    //    Closes voiced-ratio gap (28% → ~55%) without touching internal pauses.
    if (silenceTrimDb < 0) {
      const th = dbToLinear(silenceTrimDb).toFixed(6);
      // start_periods=1: only leading edge; stop_periods=-1: trailing edge
      parts.push(
        `silenceremove=start_periods=1:start_duration=0.05:start_threshold=${th}` +
        `:stop_periods=-1:stop_duration=0.1:stop_threshold=${th}:detection=rms`,
      );
    }

    // 1. Fade in / out — only if non-zero (afade=d:0 silences the signal)
    if (fadeSecs > 0) {
      parts.push(`afade=t=in:d=${fadeSecs}`);
      parts.push(`areverse,afade=t=in:d=${fadeSecs},areverse`);
    }

    // 2. Mud removal — strip sub-bass before any boosting
    if (highpassFreq > 0) {
      parts.push(`highpass=f=${highpassFreq}`);
    }

    // 2b. Spectral tilt — restore natural −6dB/oct voice roll-off flattened by RVC
    if (tiltLowDb !== 0) {
      parts.push(`lowshelf=g=${tiltLowDb}:f=100:width_type=s:width=0.7`);
    }
    if (tiltHighDb !== 0) {
      parts.push(`highshelf=g=${tiltHighDb}:f=8000:width_type=s:width=0.7`);
    }

    // 2c. De-esser — suppress metallic sibilant artifacts at 7kHz
    if (deEssDb > 0) {
      parts.push(`equalizer=f=7000:width_type=o:width=1.0:g=${-deEssDb}`);
    }

    // 2e. Vibrato — pitch micro-variation to break mechanical espeak pitch steps
    if (vibratoFreq > 0 && vibratoDepth > 0) {
      parts.push(`vibrato=f=${vibratoFreq}:d=${vibratoDepth}`);
    }

    // 2d. Breathiness — add noise floor to simulate human airflow
    if (breathinessDb < 0) {
      const ng = dbToLinear(breathinessDb);
      parts.push(`aeval=val(0)+${ng.toFixed(6)}*(random(0)-0.5)*2:c=same`);
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

    // 5b. Presence — voice intelligibility sweet spot at 2.5kHz
    if (presenceDb !== 0) {
      parts.push(`equalizer=f=2500:width_type=o:width=1.5:g=${presenceDb}`);
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

    // 9. Room reverb — spatial placement before normalize
    if (reverbMs > 0 && reverbDecay > 0) {
      parts.push(
        `aecho=${reverbInputGain}:${reverbOutputGain}:${reverbMs}:${reverbDecay}`,
      );
    }

    // 10. Loudnorm — linear=true required for single-pass pipe mode (ffmpeg 7.x).
    //     Without linear=true the two-pass algorithm applies zero gain in a pipe.
    if (normalize) {
      const lufs = rmsTargetLufs !== 0 ? rmsTargetLufs : -24;
      parts.push(`loudnorm=I=${lufs}:TP=-1:LRA=11:linear=true`);
    }

    // 11. Hard limiter — tames residual peaks after loudnorm (FON-TSK-50)
    if (limiterDb < 0) {
      const limit = dbToLinear(limiterDb).toFixed(6);
      parts.push(`alimiter=limit=${limit}:attack=5:release=50:level_in=1:level_out=1`);
    }

    // Guard: ffmpeg rejects an empty -af string. Return identity filter instead.
    return parts.length > 0 ? parts.join(",") : "anull";
  }

  async process(input: Buffer): Promise<Buffer> {
    const log = getLogger();

    const toDb = (buf: Buffer): string => {
      try {
        const { samples } = parseWav(buf);
        const r = rms(samples);
        return r > 0 ? `${(20 * Math.log10(r)).toFixed(1)} dBFS` : "-∞ dBFS";
      } catch { return "?? dBFS"; }
    };

    log.debug("SmoothingProcessor", "process start", { inputRms: toDb(input) });

    const preFilter = this.buildPreFilter();
    const padded    = ffmpegFilter(input, preFilter);
    log.debug("SmoothingProcessor", "post-pad", { rms: toDb(padded) });

    const rvcOut = await this.inner.process(padded);
    log.info("SmoothingProcessor", "post-RVC",
      { inputRms: toDb(padded), outputRms: toDb(rvcOut) });

    const postFilter = this.buildPostFilter();
    const final      = ffmpegFilter(rvcOut, postFilter);
    log.info("SmoothingProcessor", "post-filter",
      { rvcRms: toDb(rvcOut), finalRms: toDb(final) });

    return final;
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
    const log   = getLogger();
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
