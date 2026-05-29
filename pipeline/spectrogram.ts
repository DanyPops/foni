/**
 * ASCII spectrogram and power-spectral-density renderer.
 *
 * Uses the Goertzel algorithm (from audio-test-utils) to measure energy at
 * logarithmically spaced frequency bins — no external dependencies.
 *
 * Output is suitable for terminal display and test snapshots.
 */

import { goertzel } from "./audio-test-utils.ts";
import type { F0Frame } from "./voice-quality.ts";

// ─── Shared constants ─────────────────────────────────────────────────────────

/** Unicode block shading characters, ordered dark → bright. */
const SHADE_CHARS = " ·:;+=xX$#@" as const;

// ─── Spectrogram ──────────────────────────────────────────────────────────────

export interface SpectrogramOptions {
  /** Terminal columns to use for time axis. Default: 72. */
  width?: number;
  /** Rows to use for frequency axis. Default: 20. */
  height?: number;
  /** Lowest frequency to display (Hz). Default: 100. */
  minHz?: number;
  /** Highest frequency to display (Hz). Default: 4000. */
  maxHz?: number;
  /** Gamma correction for brightness mapping (< 1 → brighter, > 1 → darker). Default: 0.4. */
  gamma?: number;
  /** Title shown above the plot. */
  title?: string;
}

/**
 * Render a WAV Float32 PCM buffer as an ASCII spectrogram.
 *
 * Frequency axis is logarithmic (musically uniform); time axis is linear.
 * Each cell is the Goertzel energy at that (time, frequency) point.
 *
 * @returns multi-line string ready for console.log / snapshot
 */
export function asciiSpectrogram(
  samples:    Float32Array,
  sampleRate: number,
  opts:       SpectrogramOptions = {},
): string {
  const width  = opts.width  ?? 72;
  const height = opts.height ?? 20;
  const minHz  = opts.minHz  ?? 100;
  const maxHz  = Math.min(opts.maxHz ?? 4000, sampleRate / 2);
  const gamma  = opts.gamma  ?? 0.4;

  // frequency bins — logarithmic from minHz to maxHz, bottom=high top=low
  const freqBins = Array.from({ length: height }, (_, row) => {
    const t = row / (height - 1);                       // 0 = top (high), 1 = bottom (low)
    return maxHz * Math.pow(minHz / maxHz, t);          // exponential interpolation
  });

  // time column parameters
  const hopSize   = Math.max(1, Math.floor(samples.length / width));
  const frameSize = Math.min(512, hopSize * 2, samples.length);
  const totalMs   = samples.length / sampleRate * 1000;

  // compute energy grid  [col][row]
  let maxEnergy = 1e-12;
  const grid: number[][] = Array.from({ length: width }, () => new Array(height).fill(0) as number[]);

  for (let col = 0; col < width; col++) {
    const offset = col * hopSize;
    for (let row = 0; row < height; row++) {
      const e = goertzel(samples, freqBins[row]!, sampleRate, offset, frameSize);
      grid[col]![row] = e;
      if (e > maxEnergy) maxEnergy = e;
    }
  }

  const lines: string[] = [];

  // optional title
  if (opts.title) lines.push(opts.title);

  // spectrogram rows (frequency axis = vertical)
  for (let row = 0; row < height; row++) {
    const fHz   = freqBins[row]!;
    const label = fHz >= 1000
      ? `${(fHz / 1000).toFixed(1)}k`
      : `${Math.round(fHz)}`;
    let line = label.padStart(5) + " │";
    for (let col = 0; col < width; col++) {
      const norm = grid[col]![row]! / maxEnergy;
      const idx  = Math.floor(norm ** gamma * (SHADE_CHARS.length - 1));
      line += SHADE_CHARS[idx];
    }
    lines.push(line);
  }

  // time axis
  lines.push("      └" + "─".repeat(width));
  const midLabel = `${(totalMs / 2).toFixed(0)}ms`;
  const endLabel = `${totalMs.toFixed(0)}ms`;
  lines.push(`       0${ " ".repeat(Math.floor(width / 2) - 3) }${midLabel}${ " ".repeat(Math.ceil(width / 2) - midLabel.length - 1) }${endLabel}`);

  return lines.join("\n");
}

// ─── Power Spectral Density (PSD) ─────────────────────────────────────────────

export interface PSDOptions {
  /** Number of frequency points to sample (default 40). */
  points?:  number;
  /** Plot width in characters (default 60). */
  width?:   number;
  /** Lowest frequency (Hz). Default: 50. */
  minHz?:   number;
  /** Highest frequency (Hz). Default: 8000. */
  maxHz?:   number;
  /** Title shown above the plot. */
  title?:   string;
}

/**
 * Render power spectral density as a horizontal-bar ASCII plot.
 * Each bar = Goertzel energy at that frequency, log-scaled in dB.
 * Shows where the voice energy lives — useful for spotting missing overtones.
 */
export function asciiPSD(
  samples:    Float32Array,
  sampleRate: number,
  opts:       PSDOptions = {},
): string {
  const points = opts.points ?? 40;
  const width  = opts.width  ?? 60;
  const minHz  = opts.minHz  ?? 50;
  const maxHz  = Math.min(opts.maxHz ?? 8000, sampleRate / 2);

  // sample at logarithmically spaced frequencies
  const freqs = Array.from({ length: points }, (_, i) => {
    const t = i / (points - 1);
    return minHz * Math.pow(maxHz / minHz, t);
  });

  const energies = freqs.map(f => goertzel(samples, f, sampleRate));
  const maxE = Math.max(...energies, 1e-12);

  // convert to dB relative to max
  const dbVals = energies.map(e => e > 0 ? 20 * Math.log10(e / maxE) : -80);
  const minDb  = Math.min(...dbVals, -80);  // typically around −60 to −80

  const lines: string[] = [];
  if (opts.title) lines.push(opts.title);

  for (let i = 0; i < points; i++) {
    const f     = freqs[i]!;
    const db    = dbVals[i]!;
    const label = f >= 1000 ? `${(f / 1000).toFixed(1)}k` : `${Math.round(f)} `;
    const norm  = Math.max(0, (db - minDb) / (0 - minDb));  // 0=silent, 1=loudest
    const bars  = Math.round(norm * width);
    const dbStr = `${db.toFixed(1).padStart(7)}dB`;
    lines.push(`${label.padStart(5)} ${dbStr} ${"█".repeat(bars)}${"░".repeat(width - bars)}`);
  }

  return lines.join("\n");
}

// ─── F0 contour plot ──────────────────────────────────────────────────────────

export interface F0PlotOptions {
  /** Terminal columns. Default: 72. */
  width?:  number;
  /** Plot rows. Default: 16. */
  height?: number;
  /** Min F0 to display (Hz). Default: 50. */
  minF0?:  number;
  /** Max F0 to display (Hz). Default: 400. */
  maxF0?:  number;
  /** Title. */
  title?:  string;
}

/**
 * Render the F0 contour (pitch track) as an ASCII scatter plot.
 * Each voiced frame is a dot; unvoiced frames are gaps (dots at the bottom).
 * Reveals the quantised step pattern from espeak vs. smooth natural contour.
 */
export function asciiF0Contour(
  frames: F0Frame[],
  totalMs: number,
  opts: F0PlotOptions = {},
): string {
  const width  = opts.width  ?? 72;
  const height = opts.height ?? 16;
  const minF0  = opts.minF0  ?? 50;
  const maxF0  = opts.maxF0  ?? 400;

  // build grid
  const grid: string[][] = Array.from({ length: height }, () => Array<string>(width).fill(" "));

  for (const f of frames) {
    const col = Math.min(width - 1, Math.floor(f.timeMs / totalMs * width));
    if (!f.voiced || f.f0Hz <= 0) {
      // unvoiced: mark bottom row
      grid[height - 1]![col] = "·";
      continue;
    }
    const row = Math.max(0, Math.min(height - 1, Math.floor(
      (1 - (f.f0Hz - minF0) / (maxF0 - minF0)) * (height - 1),
    )));
    grid[row]![col] = "●";
  }

  const lines: string[] = [];
  if (opts.title) lines.push(opts.title);

  for (let row = 0; row < height; row++) {
    const f0Val = maxF0 - (row / (height - 1)) * (maxF0 - minF0);
    const label = `${Math.round(f0Val)}`.padStart(4);
    lines.push(`${label} │${grid[row]!.join("")}`);
  }
  lines.push("     └" + "─".repeat(width));
  lines.push(`      0ms${ " ".repeat(width - 8) }${totalMs.toFixed(0)}ms`);

  return lines.join("\n");
}
