/**
 * baseline.e2e.test.ts — Gap-tracking test harness.
 *
 * Synthesises each of the 3 canonical phrases through the full pipeline,
 * scores every acoustic dimension against the desired-state tensor, and
 * enforces two invariants:
 *
 *   1. Snapshot invariant — gap tables are snapshot-tested; any change
 *      (improvement OR regression) requires explicit `--update-snapshots`.
 *
 *   2. Hard ceiling — mean gap must stay below GAP_CEILING_PCT at all times;
 *      prevents catastrophic regressions from slipping through.
 *
 * Workflow:
 *   - First run: snapshots are written.
 *   - After a tuning round: `npx vitest run baseline.e2e.test.ts -u` to accept improvements.
 *   - CI: runs without -u; fails on any gap change.
 *
 * Requires: espeak-ng, RVC server at localhost:5050, baseline/target.json.
 */

import { describe, it, expect, beforeAll } from "vitest";
import { readFileSync, existsSync }          from "node:fs";
import { spawnSync }                         from "node:child_process";
import { mkdtempSync }                       from "node:fs";
import { join }                              from "node:path";
import { tmpdir }                            from "node:os";

import {
  BASELINE_PHRASES,
  computeGap,
  formatGapTable,
  formatGapSummary,
  type TargetTensor,
} from "./pipeline/gap-scorer.ts";
import { analyseVoiceBuffer }  from "./pipeline/voice-analysis.ts";
import { computeVoiceQuality } from "./pipeline/voice-quality.ts";
import { SmoothingProcessor, RVCProcessor } from "./pipeline/processors.ts";
import { DEFAULT_CONFIG }      from "./core/config.ts";

// ─── Constants ────────────────────────────────────────────────────────────────

const SAMPLE_RATE     = 22050;
/** Mean gap must stay below this at all times — hard regression ceiling. */
const GAP_CEILING_PCT = 80;

// ─── Pipeline helpers ─────────────────────────────────────────────────────────

function synthesiseEspeak(phrase: string): Buffer {
  const dir = mkdtempSync(join(tmpdir(), "foni-baseline-"));
  const out = join(dir, "out.wav");
  const r   = spawnSync(
    "espeak-ng",
    ["-v", "ru", "-s", "145", "-p", "50", "-a", "200", "-w", out, phrase],
    { encoding: "buffer" },
  );
  if (r.error || !existsSync(out)) {
    throw new Error(`espeak-ng failed: ${r.error?.message ?? "no output file"}`);
  }
  return readFileSync(out);
}

async function synthesiseFull(phrase: string): Promise<Buffer> {
  const raw  = synthesiseEspeak(phrase);
  const proc = new SmoothingProcessor(
    new RVCProcessor(DEFAULT_CONFIG.rvcUrl),
    DEFAULT_CONFIG.smoothing,
  );
  return proc.process(raw);
}

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const TENSOR_PATH = "./baseline/target.json";

let tensor: TargetTensor;
let results: ReturnType<typeof computeGap>[];

beforeAll(async () => {
  if (!existsSync(TENSOR_PATH)) {
    throw new Error(
      "baseline/target.json not found — run `npx tsx scripts/baseline-analyse.mts` first",
    );
  }
  tensor = JSON.parse(readFileSync(TENSOR_PATH, "utf8")) as TargetTensor;

  results = await Promise.all(
    BASELINE_PHRASES.map(async phrase => {
      const wav = await synthesiseFull(phrase);
      const ac  = analyseVoiceBuffer(wav, SAMPLE_RATE);
      const vq  = computeVoiceQuality(wav, SAMPLE_RATE);
      return computeGap(phrase, ac, vq, tensor);
    }),
  );
}, 120_000);

// ─── Per-phrase gap table snapshots ──────────────────────────────────────────

describe("gap table per phrase", () => {
  for (let i = 0; i < BASELINE_PHRASES.length; i++) {
    it(`snapshot — "${BASELINE_PHRASES[i]}"`, () => {
      expect(formatGapTable(results[i]!)).toMatchSnapshot();
    });
  }
});

// ─── Summary snapshot ────────────────────────────────────────────────────────

it("gap summary snapshot", () => {
  expect(formatGapSummary(results)).toMatchSnapshot();
});

// ─── Hard ceiling — catastrophic regression guard ────────────────────────────

it(`mean gap stays below ${GAP_CEILING_PCT}%`, () => {
  const meanGap = results.reduce((s, r) => s + r.meanGapPct, 0) / results.length;
  expect(meanGap).toBeLessThan(GAP_CEILING_PCT);
});

// ─── Per-dimension regression guards ─────────────────────────────────────────

it("no single dimension is at 100% gap", () => {
  const worst = results
    .flatMap(r => r.rows)
    .filter(row => row.gapPct >= 100);
  if (worst.length > 0) {
    throw new Error(
      `Dimensions at 100% gap (maximally far from target):\n` +
      worst.map(r => `  ${r.metric}: actual ${r.actual} vs target ${r.target}`).join("\n"),
    );
  }
});
