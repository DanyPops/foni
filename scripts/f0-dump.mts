/**
 * scripts/f0-dump.mts — Dump raw F0 frame data from RVC output for debugging.
 * Run: npx tsx scripts/f0-dump.mts
 */

import { readFileSync, mkdtempSync, existsSync } from "node:fs";
import { spawnSync }                             from "node:child_process";
import { join }                                  from "node:path";
import { tmpdir }                                from "node:os";
import { extractF0, f0ContourStats }             from "../pipeline/voice-quality.ts";
import { SmoothingProcessor, RVCProcessor }      from "../pipeline/processors.ts";
import { DEFAULT_CONFIG }                        from "../core/config.ts";
import { parseWav }                              from "../pipeline/audio-utils.ts";

const PHRASE      = "Ну что брат как дела";
const SAMPLE_RATE = 22050;

// Synthesise
const dir = mkdtempSync(join(tmpdir(), "f0-dump-"));
const out = join(dir, "out.wav");
spawnSync("espeak-ng", ["-v","ru","-s","145","-p","50","-a","200","-w",out, PHRASE], {encoding:"buffer"});
if (!existsSync(out)) throw new Error("espeak-ng failed");
const raw = readFileSync(out);

// Full pipeline
const proc = new SmoothingProcessor(new RVCProcessor(DEFAULT_CONFIG.rvcUrl), DEFAULT_CONFIG.smoothing);
const wav  = await proc.process(raw);
const { samples } = parseWav(wav);

// Extract with current algorithm
const frames = extractF0(samples, SAMPLE_RATE);
const voiced  = frames.filter(f => f.voiced);

console.log(`\nPhrase: "${PHRASE}"`);
console.log(`Frames: ${frames.length}  Voiced: ${voiced.length}  (${(voiced.length/frames.length*100).toFixed(0)}%)`);
console.log(`\n─── Raw F0 (Hz) for all frames (. = unvoiced) ──────────────────`);
const cols = 20;
for (let i = 0; i < frames.length; i += cols) {
  const row = frames.slice(i, i + cols).map(f =>
    f.voiced ? f.f0Hz.toFixed(0).padStart(6) : "     ."
  ).join("");
  console.log(`  t=${(i * 10).toString().padStart(4)}ms ${row}`);
}

console.log(`\n─── AC peak (voicing confidence) for voiced frames ──────────────`);
console.log(" " + voiced.slice(0, 40).map(f => f.peakCorr.toFixed(2)).join("  "));

console.log(`\n─── F0 distribution (histogram) ────────────────────────────────`);
const buckets: Record<string, number> = {};
for (const f of voiced) {
  const bucket = `${Math.round(f.f0Hz / 50) * 50}Hz`;
  buckets[bucket] = (buckets[bucket] ?? 0) + 1;
}
for (const [b, n] of Object.entries(buckets).sort((a, b) => parseInt(a[0]) - parseInt(b[0]))) {
  console.log(`  ${b.padStart(7)}: ${"█".repeat(n)}  (${n})`);
}

const stats = f0ContourStats(frames);
console.log(`\n─── Stats ───────────────────────────────────────────────────────`);
console.log(`  mean    : ${stats.meanHz.toFixed(1)} Hz`);
console.log(`  stdDev  : ${stats.stdDevHz.toFixed(1)} Hz  ← should be 20-50 for natural speech`);
console.log(`  meanΔ   : ${stats.meanDeltaHz.toFixed(1)} Hz/frame`);
console.log(`  stdDevΔ : ${stats.stdDevDelta.toFixed(1)} Hz  ← should be low for smooth contour`);
console.log(`  slope   : ${stats.slopeSemi.toFixed(2)} semitones`);
console.log(`  voiced  : ${stats.voicedCount}/${stats.totalCount} frames (${(stats.voicedRatio*100).toFixed(0)}%)`);
