/**
 * scripts/gap-report.mts — CLI wrapper around the gap-scorer infrastructure.
 *
 * All logic lives in pipeline/gap-scorer.ts.
 * This script only handles I/O: synthesise → analyse → print → persist.
 *
 * Run: npx tsx scripts/gap-report.mts
 */

import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { spawnSync }                                          from "node:child_process";
import { mkdtempSync }                                        from "node:fs";
import { join }                                               from "node:path";
import { tmpdir }                                             from "node:os";

import {
  BASELINE_PHRASES,
  computeGap,
  formatGapTable,
  formatGapSummary,
  type TargetTensor,
}                              from "../pipeline/gap-scorer.ts";
import { analyseVoiceBuffer }  from "../pipeline/voice-analysis.ts";
import { computeVoiceQuality } from "../pipeline/voice-quality.ts";
import { SmoothingProcessor, RVCProcessor, DEFAULT_SMOOTHING } from "../pipeline/processors.ts";
import { DEFAULT_CONFIG }      from "../core/config.ts";

// ─── Pipeline ─────────────────────────────────────────────────────────────────

const SAMPLE_RATE = 22050;

function synthesiseEspeak(phrase: string): Buffer {
  const dir = mkdtempSync(join(tmpdir(), "foni-gap-"));
  const out = join(dir, "out.wav");
  const r   = spawnSync(
    "espeak-ng",
    ["-v", "ru", "-s", "145", "-p", "50", "-a", "200", "-w", out, phrase],
    { encoding: "buffer" },
  );
  if (r.error || !existsSync(out)) throw new Error(`espeak-ng: ${r.error?.message ?? "no output"}`);
  return readFileSync(out);
}

async function synthesiseFull(phrase: string): Promise<Buffer> {
  const raw  = synthesiseEspeak(phrase);
  const proc = new SmoothingProcessor(
    new RVCProcessor(DEFAULT_CONFIG.rvcUrl),
    DEFAULT_SMOOTHING,
  );
  return proc.process(raw);
}

// ─── Load tensor ──────────────────────────────────────────────────────────────

const TENSOR_PATH = "./baseline/target.json";
if (!existsSync(TENSOR_PATH)) {
  console.error("❌  baseline/target.json not found — run scripts/baseline-analyse.mts first");
  process.exit(1);
}
const tensor = JSON.parse(readFileSync(TENSOR_PATH, "utf8")) as TargetTensor;

// ─── Run ──────────────────────────────────────────────────────────────────────

const results = [];

for (const phrase of BASELINE_PHRASES) {
  process.stdout.write(`▶ "${phrase}" … `);
  const wav = await synthesiseFull(phrase);
  const ac  = analyseVoiceBuffer(wav, SAMPLE_RATE);
  const vq  = computeVoiceQuality(wav, SAMPLE_RATE);
  const gap = computeGap(phrase, ac, vq, tensor);
  results.push(gap);
  console.log(`${gap.meanGapPct}% gap`);
  console.log(formatGapTable(gap));
  console.log();
}

console.log(formatGapSummary(results));

// ─── Persist ──────────────────────────────────────────────────────────────────

mkdirSync("./baseline", { recursive: true });
writeFileSync("./baseline/gap-snapshot.json", JSON.stringify({
  timestamp:    new Date().toISOString(),
  meanGapPct:   +(results.reduce((s, r) => s + r.meanGapPct, 0) / results.length).toFixed(1),
  phraseGaps:   results.map(r => ({ phrase: r.phrase, gapPct: r.meanGapPct })),
}, null, 2));
console.log("Gap snapshot written → baseline/gap-snapshot.json");
