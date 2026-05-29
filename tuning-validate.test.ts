/**
 * Tuning Validator — round 5 de-robotisation
 *
 * Synthesises every CONFIGS variant through the full pipeline,
 * runs each output through validateVoiceBuffer(), drops RED failures,
 * sorts survivors by weighted GREEN score, and prints a ranked table.
 *
 * Run:   RVC_URL=http://127.0.0.1:5050 npx vitest run tuning-validate
 * Then:  npm run listen <rank>   ← to hear a specific survivor
 */

import { describe, it, beforeAll } from "vitest";

import { EspeakBackend }      from "./backends/espeak.ts";
import { RVCProcessor, SmoothingProcessor } from "./pipeline/processors.ts";
import { validateVoiceBuffer } from "./pipeline/audio-test-utils.ts";
import type { ValidationResult } from "./pipeline/audio-test-utils.ts";
import { CONFIGS }            from "./tuning.e2e.test.ts";

const RVC_URL     = process.env.RVC_URL ?? "http://127.0.0.1:5050";
const SAMPLE_RATE = 22050; // espeak default
const PHRASE      = "Ну-ка, чики-брики и в дамке! Понял, брателло? Сейчас разберёмся.";

// ─── Helpers ─────────────────────────────────────────────────────────────────

async function isRvcReachable(): Promise<boolean> {
  try {
    const r = await fetch(`${RVC_URL}/params`, { signal: AbortSignal.timeout(2000) });
    return r.ok;
  } catch { return false; }
}

/**
 * Synthesise directly through backend → processor chain.
 * Bypasses SpeakFacade (which plays audio) — we want the raw buffer only.
 */
async function synthesiseVariant(opts: Parameters<typeof SmoothingProcessor>[1]): Promise<Buffer> {
  const backend   = new EspeakBackend("ru");
  const processor = new SmoothingProcessor(new RVCProcessor(RVC_URL), opts);
  const raw       = await backend.synthesize(PHRASE, { voice: "ru", speed: 1.15 });
  return processor.process(raw);
}

// ─── Format helpers ───────────────────────────────────────────────────────────

function fmt(n: number, digits = 2): string {
  return isFinite(n) ? n.toFixed(digits) : "−∞";
}

function bar(score: number, width = 12): string {
  const filled = Math.round(score * width);
  return "█".repeat(filled) + "░".repeat(width - filled);
}

function printTable(ranked: Array<{ name: string; label: string; result: ValidationResult }>) {
  console.log("\n╔══ Tuning Validator — Round 5 ═══════════════════════════════════════════════╗");
  console.log("║  #  Name               Score        tilt   crest  pres   hfR  noise   rms  ║");
  console.log("╠══════════════════════════════════════════════════════════════════════════════╣");
  for (const [i, { name, result: r }] of ranked.entries()) {
    const a = r.analysis;
    console.log(
      `║  ${String(i + 1).padStart(1)}  ` +
      `${name.padEnd(18).slice(0, 18)}  ` +
      `${bar(r.score)}  ` +
      `${fmt(a.spectralSlope, 1).padStart(5)}  ` +
      `${fmt(a.crestFactor).padStart(5)}  ` +
      `${fmt(a.presenceRatio, 3).padStart(5)}  ` +
      `${fmt(a.hfRatio, 3).padStart(5)}  ` +
      `${fmt(a.noiseFloorRatio, 3).padStart(5)}  ` +
      `${fmt(a.rmsDb, 1).padStart(5)}  ║`,
    );
  }
  console.log("╠══════════════════════════════════════════════════════════════════════════════╣");
  console.log("║  Ideal:  tilt 20–30  crest 15–20  pres 0.20–0.40  hfR <0.20  noise <0.04   ║");
  console.log("╚══════════════════════════════════════════════════════════════════════════════╝\n");
}

function printDropped(dropped: Array<{ name: string; reasons: string[] }>) {
  if (dropped.length === 0) return;
  console.log("  ✗ Dropped (RED failure):");
  for (const { name, reasons } of dropped) {
    console.log(`    ${name.padEnd(20)} → ${reasons.join(", ")}`);
  }
  console.log();
}

// ─── Test ─────────────────────────────────────────────────────────────────────

describe("Tuning Validator — round 5 de-robotisation", () => {
  let rvcOk = false;

  beforeAll(async () => { rvcOk = await isRvcReachable(); });

  it("validates all variants, drops RED failures, prints ranked table", async () => {
    if (!rvcOk) {
      console.log("  RVC unreachable — skipping validation");
      return;
    }

    const survivors: Array<{ name: string; label: string; result: ValidationResult }> = [];
    const dropped:   Array<{ name: string; reasons: string[] }> = [];

    for (const { name, label, opts } of CONFIGS) {
      process.stdout.write(`  Synthesising ${name}... `);
      const buf    = await synthesiseVariant(opts);
      const result = validateVoiceBuffer(buf, SAMPLE_RATE);

      if (!result.passed) {
        process.stdout.write(`DROP [${result.redFailures.join(", ")}]\n`);
        dropped.push({ name, reasons: result.redFailures });
      } else {
        process.stdout.write(`score=${result.score.toFixed(3)} ✓\n`);
        survivors.push({ name, label, result });
      }
    }

    survivors.sort((a, b) => b.result.score - a.result.score);

    printDropped(dropped);
    printTable(survivors);

    console.log("  Ranked listen order:");
    for (const [i, { name, label }] of survivors.entries()) {
      console.log(`    ${i + 1}. ${name.padEnd(18)} ${label}`);
    }
    console.log("\n  Run: npm run listen <number above>\n");

  }, 5 * 60 * 1000);
});
