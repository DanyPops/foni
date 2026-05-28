/**
 * DSP unit tests — acoustic assertions for each SmoothingProcessor stage.
 *
 * Design:
 *   - IdentityProcessor as inner → isolates DSP only, no RVC
 *   - padSecs:0 in isolation tests → no silence prefix to skip
 *   - Windowed Goertzel (2048 samples) → avoids catastrophic cancellation
 *   - Pure sine waves → deterministic, exact frequency content
 *   - Gate: only play if acoustic assertion passes
 *
 * Run silent:   npx vitest run dsp
 * Run + play:   FONI_PLAY=1 npx vitest run dsp
 */

import { describe, it, expect } from "vitest";
import {
  SmoothingProcessor, IdentityProcessor, DEFAULT_SMOOTHING,
} from "./pipeline/processors.ts";
import type { SmoothingOptions }     from "./pipeline/processors.ts";
import { SystemPlayer }              from "./pipeline/player.ts";
import {
  generateSineWav, generateNoiseWav, parseWav,
  rms, peak, goertzel, dbChange, bandEnergy, spectralCentroid,
} from "./pipeline/audio-test-utils.ts";

const PLAY = process.env.FONI_PLAY === "1";
const RATE = 22050;
const DUR  = 1.5;   // seconds — enough cycles for all test frequencies

// ─── Helpers ─────────────────────────────────────────────────────────────────

/** Isolation preset: disable everything except what's being tested. */
const ISO: Partial<SmoothingOptions> = {
  padSecs:          0,        // no silence padding — simpler output analysis
  fadeSecs:         0,
  highpassFreq:     0,
  deBoxDb:          0,
  deHarshDb:        0,
  warmthBoostDb:    0,
  airBoostDb:       0,
  eqGain:           0,
  saturationDrive:  0,
  saturationAmount: 0,
  phaserDepth:      0,
  reverbMs:         0,
  compressionRatio: 1,
  normalize:        false,
};

function makeProcessor(opts: Partial<SmoothingOptions>) {
  return new SmoothingProcessor(new IdentityProcessor(), {
    ...DEFAULT_SMOOTHING,
    ...opts,
  });
}

/**
 * Measure energy at freqHz from a WAV buffer.
 * padOffset: samples to skip at the start (for silence padding).
 */
function energy(wav: Buffer, freqHz: number, padOffset = 0): number {
  const { samples, sampleRate } = parseWav(wav);
  return goertzel(samples, freqHz, sampleRate, padOffset);
}

async function playIfPass(buf: Buffer, label: string) {
  if (!PLAY) return;
  const player = new SystemPlayer();
  if (!player.detected()) return;
  console.info(`  ▶  ${label}`);
  await player.play(buf);
}

// ─── Highpass ─────────────────────────────────────────────────────────────────

describe("highpass filter", () => {
  it("attenuates sub-cutoff (40Hz) by > 10dB", async () => {
    const wav  = generateSineWav(40, DUR, RATE);
    const proc = makeProcessor({ ...ISO, highpassFreq: 80 });
    const out  = await proc.process(wav);

    const inAmp  = energy(wav, 40);
    const outAmp = energy(out, 40);
    const cutDb  = dbChange(inAmp, outAmp);

    console.info(`  [highpass] 40Hz: ${cutDb.toFixed(1)}dB (expect < -10dB)`);
    expect(cutDb).toBeLessThan(-10);
    await playIfPass(out, `40Hz sine through highpass(80Hz): ${cutDb.toFixed(0)}dB`);
  });

  it("passes 400Hz (well above cutoff) within 4dB", async () => {
    const wav  = generateSineWav(400, DUR, RATE);
    const proc = makeProcessor({ ...ISO, highpassFreq: 80 });
    const out  = await proc.process(wav);

    const inAmp  = energy(wav, 400);
    const outAmp = energy(out, 400);
    const db     = dbChange(inAmp, outAmp);

    console.info(`  [highpass] 400Hz: ${db.toFixed(1)}dB (expect > -4dB)`);
    expect(db).toBeGreaterThan(-4);
  });
});

// ─── deBox EQ cut ─────────────────────────────────────────────────────────────

describe("deBox EQ cut", () => {
  it("cuts target frequency by at least (deBoxDb - 1)dB", async () => {
    const freq  = 300;
    const cutDb = -4;
    const wav   = generateSineWav(freq, DUR, RATE);
    const proc  = makeProcessor({
      ...ISO,
      deBoxFreq: freq, deBoxDb: cutDb, deBoxBandwidthOctaves: 1.5,
    });
    const out = await proc.process(wav);

    const inAmp  = energy(wav, freq);
    const outAmp = energy(out, freq);
    const actual = dbChange(inAmp, outAmp);

    console.info(`  [deBox] ${freq}Hz: ${actual.toFixed(1)}dB (expect ≤ ${cutDb + 1}dB)`);
    expect(actual).toBeLessThan(cutDb + 1);
    await playIfPass(out, `deBox: ${freq}Hz cut → ${actual.toFixed(1)}dB`);
  });

  it("does not cut a frequency 2 octaves away (1200Hz)", async () => {
    const wav  = generateSineWav(1200, DUR, RATE);
    const proc = makeProcessor({
      ...ISO,
      deBoxFreq: 300, deBoxDb: -4, deBoxBandwidthOctaves: 1.5,
    });
    const out = await proc.process(wav);

    const inAmp  = energy(wav, 1200);
    const outAmp = energy(out, 1200);
    const db     = dbChange(inAmp, outAmp);

    console.info(`  [deBox] 1200Hz (far away): ${db.toFixed(1)}dB (expect > -3dB)`);
    expect(db).toBeGreaterThan(-3);
  });
});

// ─── Warmth boost ─────────────────────────────────────────────────────────────

describe("warmth low-shelf boost", () => {
  it("boosts target frequency by at least 0.8dB", async () => {
    const freq  = 200;
    const boost = 2.5;
    const wav   = generateSineWav(freq, DUR, RATE);
    const proc  = makeProcessor({
      ...ISO, warmthBoostDb: boost, warmthFreq: freq,
    });
    const out = await proc.process(wav);

    const inAmp  = energy(wav, freq);
    const outAmp = energy(out, freq);
    const actual = dbChange(inAmp, outAmp);

    // Shelf filter applies gradually — measured at corner freq expect ~half gain
    console.info(`  [warmth] ${freq}Hz: ${actual.toFixed(1)}dB (expect > +0.8dB)`);
    expect(actual).toBeGreaterThan(0.8);
    await playIfPass(out, `warmth: ${freq}Hz +${boost}dB → ${actual.toFixed(1)}dB`);
  });

  it("warmthBoostDb=0 makes no significant change at 200Hz (within 2dB)", async () => {
    const wav  = generateSineWav(200, DUR, RATE);
    const proc = makeProcessor({ ...ISO, warmthBoostDb: 0 });
    const out  = await proc.process(wav);

    const db = dbChange(energy(wav, 200), energy(out, 200));
    console.info(`  [warmth=0] 200Hz: ${db.toFixed(1)}dB (expect |db| < 2)`);
    expect(Math.abs(db)).toBeLessThan(2);
  });
});

// ─── Presence EQ boost ────────────────────────────────────────────────────────

describe("presence EQ boost (2–5kHz)", () => {
  it("boosts target frequency via deHarshDb > 0", async () => {
    const freq  = 3000;
    const boost = 3;
    const wav   = generateSineWav(freq, DUR, RATE);
    // deHarshDb positive = boost (same peaking EQ, just positive gain)
    const proc  = makeProcessor({
      ...ISO, deHarshFreq: freq, deHarshDb: boost, deHarshBandwidthOctaves: 2,
    });
    const out = await proc.process(wav);

    const inAmp  = energy(wav, freq);
    const outAmp = energy(out, freq);
    const actual = dbChange(inAmp, outAmp);

    console.info(`  [presence] ${freq}Hz: ${actual.toFixed(1)}dB (expect ≥ +${boost - 1}dB)`);
    expect(actual).toBeGreaterThan(boost - 1);
    await playIfPass(out, `presence: ${freq}Hz +${boost}dB → ${actual.toFixed(1)}dB`);
  });

  it("deHarshDb negative cuts target frequency", async () => {
    const freq = 3500;
    const cut  = -2;
    const wav  = generateSineWav(freq, DUR, RATE);
    const proc = makeProcessor({
      ...ISO, deHarshFreq: freq, deHarshDb: cut, deHarshBandwidthOctaves: 2,
    });
    const out = await proc.process(wav);

    const actual = dbChange(energy(wav, freq), energy(out, freq));
    console.info(`  [EQ cut] ${freq}Hz: ${actual.toFixed(1)}dB (expect ≤ ${cut + 1}dB)`);
    expect(actual).toBeLessThan(cut + 1);
  });
});

// ─── Air high-shelf ──────────────────────────────────────────────────────────

describe("air high-shelf boost", () => {
  it("boosts 8kHz by at least (airBoostDb - 1)dB", async () => {
    const freq  = 8000;
    const boost = 2;
    const wav   = generateSineWav(freq, DUR, RATE);
    const proc  = makeProcessor({
      ...ISO, airBoostDb: boost, airFreq: 6000,
    });
    const out = await proc.process(wav);

    const actual = dbChange(energy(wav, freq), energy(out, freq));
    console.info(`  [air] ${freq}Hz: ${actual.toFixed(1)}dB (expect ≥ +${boost - 1}dB)`);
    expect(actual).toBeGreaterThan(boost - 1);
    await playIfPass(out, `air: ${freq}Hz +${boost}dB → ${actual.toFixed(1)}dB`);
  });
});

// ─── Compressor ──────────────────────────────────────────────────────────────

describe("compressor", () => {
  it("reduces RMS of a loud signal (ratio 4:1, makeup=0)", async () => {
    const wav  = generateSineWav(1000, DUR, RATE, 0.9);
    const proc = makeProcessor({
      ...ISO,
      compressionRatio:       4,
      compressionThresholdDb: -12,
      compressionMakeupDb:    0,
      normalize:              false,
    });
    const out = await proc.process(wav);

    const inRms  = rms(parseWav(wav).samples);
    const outRms = rms(parseWav(out).samples);
    const change = dbChange(inRms, outRms);

    console.info(`  [compressor] RMS: ${inRms.toFixed(3)} → ${outRms.toFixed(3)} (${change.toFixed(1)}dB)`);
    expect(outRms).toBeLessThan(inRms);
    await playIfPass(out, `compressor 4:1: RMS ${change.toFixed(1)}dB`);
  });

  it("does not clip with moderate makeup gain (peak < 0.99)", async () => {
    // Use quieter input + modest makeup to avoid saturation
    const wav  = generateSineWav(1000, DUR, RATE, 0.6);
    const proc = makeProcessor({ ...ISO, compressionRatio: 3, compressionMakeupDb: 2, normalize: false });
    const out  = await proc.process(wav);
    const pk   = peak(parseWav(out).samples);

    console.info(`  [compressor+makeup] peak: ${pk.toFixed(3)} (expect < 0.99)`);
    expect(pk).toBeLessThan(0.99);
  });
});

// ─── Loudnorm ────────────────────────────────────────────────────────────────

describe("loudnorm", () => {
  it("raises RMS of a quiet signal", async () => {
    const wav  = generateSineWav(440, DUR, RATE, 0.05);  // very quiet
    const proc = makeProcessor({ ...ISO, normalize: true });
    const out  = await proc.process(wav);

    const inRms  = rms(parseWav(wav).samples);
    const outRms = rms(parseWav(out).samples);
    const change = dbChange(inRms, outRms);

    console.info(`  [loudnorm] RMS: ${inRms.toFixed(4)} → ${outRms.toFixed(4)} (${change.toFixed(1)}dB gain)`);
    expect(outRms).toBeGreaterThan(inRms);  // must be louder
    expect(change).toBeGreaterThan(3);      // meaningfully louder, not just rounding
    await playIfPass(out, `loudnorm: quiet sine +${change.toFixed(0)}dB`);
  });
});

// ─── Fade in/out ─────────────────────────────────────────────────────────────

describe("fade in/out", () => {
  it("first 5ms of output is near silence (fade-in)", async () => {
    const wav  = generateSineWav(440, DUR, RATE, 0.8);
    const proc = makeProcessor({ ...ISO, fadeSecs: 0.05, normalize: false });
    const out  = await proc.process(wav);

    const { samples } = parseWav(out);
    const first5ms    = samples.slice(0, Math.floor(RATE * 0.005));
    const firstRms    = rms(first5ms);

    console.info(`  [fade] first 5ms RMS: ${firstRms.toFixed(5)} (expect < 0.05)`);
    expect(firstRms).toBeLessThan(0.05);
    await playIfPass(out, `fade in/out 50ms: 440Hz sine`);
  });
});

// ─── Full chain: spectral properties ─────────────────────────────────────────

describe("full DSP chain", () => {
  it("de-mud config cuts 300Hz more than baseline", async () => {
    const wav = generateSineWav(300, DUR, RATE);

    const baseline = makeProcessor({ ...ISO });
    const demud    = makeProcessor({
      ...ISO,
      deBoxFreq: 300, deBoxDb: -4, deBoxBandwidthOctaves: 1.5,
    });

    const outBase  = await baseline.process(wav);
    const outDemud = await demud.process(wav);

    const dbBase  = dbChange(energy(wav, 300), energy(outBase, 300));
    const dbDemud = dbChange(energy(wav, 300), energy(outDemud, 300));

    console.info(`  [de-mud 300Hz] baseline: ${dbBase.toFixed(1)}dB  de-mud: ${dbDemud.toFixed(1)}dB`);
    expect(dbDemud).toBeLessThan(dbBase - 2);  // de-mud must cut at least 2dB more

    if (PLAY) {
      const player = new SystemPlayer();
      if (player.detected()) {
        console.info("  ▶ baseline 300Hz");
        await player.play(outBase);
        await new Promise(r => setTimeout(r, 400));
        console.info("  ▶ de-mud 300Hz");
        await player.play(outDemud);
      }
    }
  });

  it("presence boost raises 3kHz more than baseline", async () => {
    const wav = generateSineWav(3000, DUR, RATE);

    const baseline = makeProcessor({ ...ISO });
    const present  = makeProcessor({
      ...ISO, deHarshFreq: 3000, deHarshDb: 3, deHarshBandwidthOctaves: 2,
    });

    const dbBase    = dbChange(energy(wav, 3000), energy(await baseline.process(wav), 3000));
    const dbPresent = dbChange(energy(wav, 3000), energy(await present.process(wav), 3000));

    console.info(`  [presence 3kHz] baseline: ${dbBase.toFixed(1)}dB  boosted: ${dbPresent.toFixed(1)}dB`);
    expect(dbPresent).toBeGreaterThan(dbBase + 2);

    if (PLAY) {
      const player = new SystemPlayer();
      if (player.detected()) {
        console.info("  ▶ baseline 3kHz");
        await player.play(await makeProcessor({ ...ISO }).process(wav));
        await new Promise(r => setTimeout(r, 400));
        console.info("  ▶ presence +3dB 3kHz");
        await player.play(await present.process(wav));
      }
    }
  });

  it("output is not clipped on loud noise (peak < 0.99)", async () => {
    const wav  = generateNoiseWav(DUR, RATE, 0.8);
    const proc = makeProcessor({});  // DEFAULT_SMOOTHING — full chain
    const out  = await proc.process(wav);
    const pk   = peak(parseWav(out).samples);

    console.info(`  [full chain] noise peak: ${pk.toFixed(3)} (expect < 0.99)`);
    expect(pk).toBeLessThan(0.99);
    await playIfPass(out, `full chain on noise — peak ${pk.toFixed(3)}`);
  });
});
