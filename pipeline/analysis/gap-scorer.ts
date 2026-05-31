/**
 * pipeline/gap-scorer.ts — Desired-state gap scoring infrastructure.
 *
 * Pure functions, no I/O, no side effects.
 * Consumed by baseline.e2e.test.ts and scripts/gap-report.mts.
 *
 * Architecture:
 *   TargetTensor   — loaded from baseline/target.json (authored by baseline-analyse.mts)
 *   GapRow         — one metric's distance from target
 *   GapResult      — full scored result for one audio buffer
 *   computeGap()   — scores a buffer against the tensor
 *   formatGapTable() — renders a human-readable table (used in snapshots)
 */

import type { AudioAnalysis }      from "./voice-analysis.ts";
import type { VoiceQualityMetrics } from "./voice-quality.ts";

// ─── Shared test constants ────────────────────────────────────────────────────

/**
 * The 3 canonical phrases used for all gap measurements.
 * Different register + length to prevent over-fitting to a single utterance.
 */
export const BASELINE_PHRASES = [
  "Давай, шевелись.",          // short command
  "Ну что, брат, как дела?",   // conversational greeting
  "Слушай, тут такое дело.",   // opener, mid-length
] as const;

export type BaselinePhrase = (typeof BASELINE_PHRASES)[number];

/** Paths to the 3 STALKER WAV files that define the desired-state tensor. */
export const BASELINE_WAV_FILES = [
  "baseline/stalker/wav/sidorovich/trader1a.wav",
  "baseline/stalker/wav/cherevatenko/zat_a2_stalker_barmen_greeting_1.wav",
  "baseline/stalker/wav/cherevatenko/zat_a2_stalker_barmen_farewell_1.wav",
] as const;

// ─── Tensor type ─────────────────────────────────────────────────────────────

export interface TargetTensor {
  _description: string;
  _sources:     string[];
  voice: {
    f0MeanHz:        number;
    f0StdDevHz:      number;
    f0SlopeSemi:     number;
    f0DeltaSigmaHz:  number;
    voicedRatio:     number;
    hnrDbMean:       number;
    hnrDbMax:        number;
    cppMean:         number;
    jitter:          number;
    shimmer:         number;
  };
  spectral: {
    rmsDb:           number;
    spectralSlope:   number;
    spectralTilt:    number;
    crestFactor:     number;
    presenceRatio:   number;
    hfRatio:         number;
    lfRatio:         number;
    noiseFloorRatio: number;
  };
  roboticness: {
    targetScore: number;
  };
}

// ─── Gap types ────────────────────────────────────────────────────────────────

export type Verdict = "✅ close" | "🟡 near" | "🟠 far" | "🔴 very far";

export interface GapRow {
  metric:  string;
  target:  string;
  actual:  string;
  gapPct:  number;     // 0–100
  verdict: Verdict;
}

export interface GapResult {
  phrase:      string;
  rows:        GapRow[];
  meanGapPct:  number;
}

// ─── Metric definitions ───────────────────────────────────────────────────────

interface MetricDef {
  name:   string;
  target: (t: TargetTensor) => number;
  actual: (ac: AudioAnalysis, vq: VoiceQualityMetrics) => number;
  unit:   string;
  scale:  number;   // normalisation range — 100% gap = this many units away
}

const METRICS: MetricDef[] = [
  {
    name:   "RMS level",
    target: t => t.spectral.rmsDb,
    actual: (ac)    => ac.rmsDb,
    unit:   " dBFS",
    scale:  10,
  },
  {
    name:   "Crest factor",
    target: t => t.spectral.crestFactor,
    actual: (ac)    => ac.crestFactor,
    unit:   " dB",
    scale:  10,
  },
  {
    name:   "Spectral slope",
    target: t => t.spectral.spectralSlope,
    actual: (ac)    => ac.spectralSlope,
    unit:   " dB/oct",
    scale:  10,
  },
  {
    name:   "Spectral tilt",
    target: t => t.spectral.spectralTilt,
    actual: (ac)    => ac.spectralTilt,
    unit:   " dB",
    scale:  40,
  },
  {
    name:   "Noise floor",
    target: t => t.spectral.noiseFloorRatio,
    actual: (ac)    => ac.noiseFloorRatio,
    unit:   "",
    scale:  0.1,
  },
  {
    name:   "Presence ratio",
    target: t => t.spectral.presenceRatio,
    actual: (ac)    => ac.presenceRatio,
    unit:   "",
    scale:  0.3,
  },
  {
    name:   "Voiced ratio",
    target: t => t.voice.voicedRatio,
    actual: (_ac, vq) => vq.voicedRatio,
    unit:   "",
    scale:  0.5,
  },
  {
    name:   "F0 stdDev",
    target: t => t.voice.f0StdDevHz,
    actual: (_ac, vq) => vq.f0Stats.stdDevHz,
    unit:   " Hz",
    scale:  150,
  },
];

// ─── Core scoring function ────────────────────────────────────────────────────

function verdict(gapPct: number): Verdict {
  if (gapPct < 15) return "✅ close";
  if (gapPct < 35) return "🟡 near";
  if (gapPct < 60) return "🟠 far";
  return "🔴 very far";
}

/**
 * Score one synthesised audio buffer against the desired-state tensor.
 * Both AudioAnalysis and VoiceQualityMetrics must come from the same buffer.
 */
export function computeGap(
  phrase: string,
  ac:     AudioAnalysis,
  vq:     VoiceQualityMetrics,
  tensor: TargetTensor,
): GapResult {
  const rows: GapRow[] = METRICS.map(m => {
    const tVal   = m.target(tensor);
    const aVal   = m.actual(ac, vq);
    const gapPct = Math.min(100, Math.abs(aVal - tVal) / m.scale * 100);
    return {
      metric:  m.name,
      target:  `${tVal.toFixed(2)}${m.unit}`,
      actual:  `${aVal.toFixed(2)}${m.unit}`,
      gapPct:  +gapPct.toFixed(1),
      verdict: verdict(gapPct),
    };
  });

  const meanGapPct = +(rows.reduce((s, r) => s + r.gapPct, 0) / rows.length).toFixed(1);
  return { phrase, rows, meanGapPct };
}

// ─── Formatting ───────────────────────────────────────────────────────────────

/** Render a gap result as a fixed-width table string (used in snapshots). */
export function formatGapTable(r: GapResult): string {
  const header = [
    `Phrase: "${r.phrase}"`,
    `${"Metric".padEnd(18)} ${"Target".padEnd(14)} ${"Actual".padEnd(14)} Gap%   Verdict`,
    "─".repeat(66),
  ];
  const body = r.rows.map(row =>
    `${row.metric.padEnd(18)} ${row.target.padEnd(14)} ${row.actual.padEnd(14)} ` +
    `${String(row.gapPct).padStart(4)}%  ${row.verdict}`,
  );
  return [...header, ...body, "─".repeat(66), `Mean gap: ${r.meanGapPct}%`].join("\n");
}

/** Render a summary table across multiple phrases. */
export function formatGapSummary(results: GapResult[]): string {
  const meanGap = +(results.reduce((s, r) => s + r.meanGapPct, 0) / results.length).toFixed(1);

  // Per-dimension averages
  const dimGaps = new Map<string, number[]>();
  for (const r of results) {
    for (const row of r.rows) {
      const arr = dimGaps.get(row.metric) ?? [];
      arr.push(row.gapPct);
      dimGaps.set(row.metric, arr);
    }
  }
  const ranked = [...dimGaps.entries()]
    .map(([m, gs]) => ({ m, avg: +(gs.reduce((a, b) => a + b, 0) / gs.length).toFixed(1) }))
    .sort((a, b) => b.avg - a.avg);

  const lines = [
    "═".repeat(66),
    "  BASELINE GAP SUMMARY",
    "─".repeat(66),
    ...results.map(r => `  "${r.phrase}"  →  ${r.meanGapPct}%`),
    "─".repeat(66),
    `  Mean gap: ${meanGap}%`,
    "",
    "  Worst dimensions:",
    ...ranked.map(({ m, avg }) => {
      const bar = "█".repeat(Math.round(avg / 5)).padEnd(20, "░");
      return `    ${m.padEnd(18)} ${bar} ${avg}%`;
    }),
    "═".repeat(66),
  ];
  return lines.join("\n");
}

// ─── Regression helpers ───────────────────────────────────────────────────────

/** Maximum allowable gap regression between test runs before failing. */
export const REGRESSION_TOLERANCE_PCT = 5;

/**
 * Assert that mean gap has not regressed beyond tolerance.
 * Throws if regressionPct > REGRESSION_TOLERANCE_PCT.
 */
export function assertNoRegression(
  currentMean:  number,
  snapshotMean: number,
  label         = "",
): void {
  const delta = currentMean - snapshotMean;
  if (delta > REGRESSION_TOLERANCE_PCT) {
    throw new Error(
      `${label} gap REGRESSED: ${snapshotMean}% → ${currentMean}% ` +
      `(+${delta.toFixed(1)}% exceeds tolerance of ${REGRESSION_TOLERANCE_PCT}%)`,
    );
  }
}
