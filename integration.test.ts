/**
 * integration.test.ts — Rust↔TS seam tests.
 *
 * Verifies that the TypeScript extension correctly calls foni-synth endpoints
 * and that those calls produce measurable effects on the audio.
 *
 * Skipped when FONI_SYNTH_URL is not set (local dev without foni-synth running).
 * Required in CI with foni-synth started before the test run.
 *
 * Run with foni-synth:
 *   cargo run -p foni-synth &
 *   FONI_SYNTH_URL=http://localhost:5050 npx vitest run integration
 */

import { describe, it, expect, beforeAll } from "vitest";
import { SmoothingProcessor, IdentityProcessor } from "./pipeline/processors.ts";
import { BreathProcessor, injectBreaths }          from "./pipeline/breath-injector.ts";
import { parseWav, rms }                           from "./pipeline/analysis/audio-utils.ts";
import { generateSineWav }                         from "./pipeline/analysis/audio-test-utils.ts";

const SYNTH_URL  = process.env.FONI_SYNTH_URL;
const SKIP       = !SYNTH_URL;
const SR         = 22050;

// ─── Helpers ─────────────────────────────────────────────────────────────────

async function serverReachable(url: string): Promise<boolean> {
  try {
    const r = await fetch(`${url}/models`, { signal: AbortSignal.timeout(2000) });
    return r.ok;
  } catch { return false; }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe.skipIf(SKIP)("Rust↔TS integration — foni-synth HTTP seam", () => {
  beforeAll(async () => {
    if (!SYNTH_URL) return;
    const ok = await serverReachable(SYNTH_URL);
    if (!ok) throw new Error(`foni-synth not reachable at ${SYNTH_URL} — start it first`);
  });

  it("POST /process changes audio bytes (DSP chain applied)", async () => {
    const input = generateSineWav(440, 0.5, SR, 0.5);
    const proc  = new SmoothingProcessor(new IdentityProcessor(), {}, SYNTH_URL!);
    const output = await proc.process(input);

    expect(output.length).toBeGreaterThan(44);
    expect(output.equals(input)).toBe(false);

    // Loudnorm must change the RMS
    const { samples: inSamples }  = parseWav(input);
    const { samples: outSamples } = parseWav(output);
    const rmsIn  = rms(inSamples);
    const rmsOut = rms(outSamples);
    console.log(`  RMS: in=${(20*Math.log10(rmsIn)).toFixed(1)}dBFS  out=${(20*Math.log10(rmsOut)).toFixed(1)}dBFS`);
    expect(Math.abs(rmsIn - rmsOut)).toBeGreaterThan(0.001);
  });

  it("POST /breath returns non-silent audio (Rust bandpass noise)", async () => {
    const silentWav = generateSineWav(0, 0, SR, 0); // silent WAV
    const result    = await injectBreaths(silentWav, SR, {}, SYNTH_URL!);
    // When no gaps found, result equals input — just check /breath is callable
    const resp = await fetch(`${SYNTH_URL}/breath`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ duration_ms: 120, sample_rate: SR }),
    });
    expect(resp.ok).toBe(true);
    const { audio_data } = await resp.json() as { audio_data: string };
    const buf = Buffer.from(audio_data, "base64");
    const { samples } = parseWav(buf);
    const breathRms = rms(samples);
    console.log(`  Breath RMS: ${breathRms.toFixed(6)}`);
    expect(breathRms).toBeGreaterThan(1e-4);
  });

  it("POST /analyse returns valid AnalysisResult", async () => {
    const wav  = generateSineWav(440, 0.5, SR, 0.5);
    const resp = await fetch(`${SYNTH_URL}/analyse`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ audio_data: wav.toString("base64") }),
    });
    expect(resp.ok).toBe(true);
    const body = await resp.json() as { analysis: { loudness: { rms_db: number }; temporal: { duration_secs: number } } };
    const { rms_db } = body.analysis.loudness;
    const { duration_secs } = body.analysis.temporal;
    console.log(`  Analysis: rms=${rms_db.toFixed(1)}dBFS  dur=${duration_secs.toFixed(3)}s`);
    expect(rms_db).toBeLessThan(0);
    expect(rms_db).toBeGreaterThan(-60);
    expect(Math.abs(duration_secs - 0.5)).toBeLessThan(0.05);
  });

  it("FONI_REQUIRE_DSP=1 causes SmoothingProcessor to throw when unreachable", async () => {
    process.env.FONI_REQUIRE_DSP = "1";
    const proc = new SmoothingProcessor(new IdentityProcessor(), {}, "http://localhost:19999");
    const wav  = generateSineWav(440, 0.1, SR, 0.5);
    await expect(proc.process(wav)).rejects.toThrow(/unreachable/);
    delete process.env.FONI_REQUIRE_DSP;
  });
});

// ─── Always-on: FONI_REQUIRE_DSP throw behaviour ─────────────────────────────

describe("SmoothingProcessor fallback behaviour", () => {
  it("silent fallback when FONI_REQUIRE_DSP unset and server unreachable", async () => {
    const proc = new SmoothingProcessor(new IdentityProcessor(), {}, "http://localhost:19999");
    const wav  = generateSineWav(440, 0.1, SR, 0.5);
    // Must NOT throw — returns rvcOut unchanged
    const out = await proc.process(wav);
    expect(out.length).toBeGreaterThan(44);
  });

  it("throws when FONI_REQUIRE_DSP=1 and server unreachable", async () => {
    process.env.FONI_REQUIRE_DSP = "1";
    const proc = new SmoothingProcessor(new IdentityProcessor(), {}, "http://localhost:19999");
    const wav  = generateSineWav(440, 0.1, SR, 0.5);
    await expect(proc.process(wav)).rejects.toThrow();
    delete process.env.FONI_REQUIRE_DSP;
  });
});
