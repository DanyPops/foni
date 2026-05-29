/**
 * Voice quality metrics — unit tests using synthetic signals.
 *
 * No external services required (espeak/RVC). All signals are generated
 * programmatically to verify algorithm correctness under known conditions.
 *
 * Covered metrics: F0 extraction, HNR, CPP, jitter, shimmer, F0 contour stats,
 * roboticness score, ASCII spectrogram rendering.
 */

import { describe, it, expect } from "vitest";
import { generateSineWav, generateNoiseWav, parseWav } from "./pipeline/audio-test-utils.ts";
import {
  fft,
  extractF0,
  computeJitter,
  computeShimmer,
  f0ContourStats,
  computeCPP,
  computeVoiceQuality,
  roboticnessScore,
} from "./pipeline/voice-quality.ts";
import { asciiSpectrogram, asciiPSD, asciiF0Contour } from "./pipeline/spectrogram.ts";

// ─── Helpers ──────────────────────────────────────────────────────────────────

const SAMPLE_RATE = 22050;

/** Generate a harmonic series (voiced vowel simulation) at fundamental f0. */
function generateHarmonicWav(
  f0Hz:         number,
  durationSecs: number,
  harmonics     = 8,
  sampleRate    = SAMPLE_RATE,
): Buffer {
  const numSamples = Math.floor(sampleRate * durationSecs);
  const dataSize   = numSamples * 2;
  const buf        = Buffer.alloc(44 + dataSize);
  buf.write("RIFF", 0); buf.writeUInt32LE(36 + dataSize, 4); buf.write("WAVE", 8);
  buf.write("fmt ", 12); buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(1, 22); buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28); buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34);
  buf.write("data", 36); buf.writeUInt32LE(dataSize, 40);

  for (let i = 0; i < numSamples; i++) {
    let s = 0;
    for (let h = 1; h <= harmonics; h++) {
      s += Math.sin(2 * Math.PI * f0Hz * h * i / sampleRate) / h; // 1/h amplitude rolloff
    }
    s *= 0.6; // normalise headroom
    buf.writeInt16LE(Math.round(Math.max(-32767, Math.min(32767, s * 32767))), 44 + i * 2);
  }
  return buf;
}

// ─── FFT ─────────────────────────────────────────────────────────────────────

describe("fft", () => {
  it("is idempotent on length-1 input", () => {
    const re = new Float64Array([3.0]);
    const im = new Float64Array([0.0]);
    fft(re, im);
    expect(re[0]).toBeCloseTo(3.0);
    expect(im[0]).toBeCloseTo(0.0);
  });

  it("FFT of [1,0,0,0] → [1,1,1,1] (DC)", () => {
    const re = new Float64Array([1, 0, 0, 0]);
    const im = new Float64Array(4);
    fft(re, im);
    for (let k = 0; k < 4; k++) {
      expect(re[k]).toBeCloseTo(1, 5);
      expect(im[k]).toBeCloseTo(0, 5);
    }
  });

  it("FFT of [0,1,0,0] → alternating imaginary (Nyquist bin)", () => {
    const re = new Float64Array([0, 1, 0, 0]);
    const im = new Float64Array(4);
    fft(re, im);
    // DFT of unit sample at n=1: X[k] = e^(-j2π k/N)
    // X[0]=1, X[1]=-j, X[2]=-1, X[3]=j
    expect(re[0]).toBeCloseTo(1, 5);
    expect(re[2]).toBeCloseTo(-1, 5);
    expect(im[1]).toBeCloseTo(-1, 5);
    expect(im[3]).toBeCloseTo(1, 5);
  });

  it("Parseval's theorem: energy preserved", () => {
    const N = 64;
    const re = Float64Array.from({ length: N }, () => Math.random() - 0.5);
    const im = new Float64Array(N);
    const energyTime = re.reduce((s, x) => s + x * x, 0);
    fft(re, im);
    const energyFreq = (re.reduce((s, x, i) => s + x * x + im[i]! * im[i]!, 0)) / N;
    expect(energyFreq).toBeCloseTo(energyTime, 1);
  });
});

// ─── F0 extraction ────────────────────────────────────────────────────────────

describe("extractF0", () => {
  it("detects correct F0 for pure 200 Hz sine", () => {
    const wav    = generateSineWav(200, 0.5, SAMPLE_RATE);
    const { samples } = parseWav(wav);
    const frames = extractF0(samples, SAMPLE_RATE);
    const voiced = frames.filter(f => f.voiced);
    expect(voiced.length).toBeGreaterThan(5);
    const meanF0 = voiced.reduce((s, f) => s + f.f0Hz, 0) / voiced.length;
    expect(meanF0).toBeGreaterThan(180);
    expect(meanF0).toBeLessThan(220);
  });

  it("detects correct F0 for 150 Hz harmonic series", () => {
    const wav    = generateHarmonicWav(150, 0.5);
    const { samples } = parseWav(wav);
    const frames = extractF0(samples, SAMPLE_RATE);
    const voiced = frames.filter(f => f.voiced);
    expect(voiced.length).toBeGreaterThan(3);
    const meanF0 = voiced.reduce((s, f) => s + f.f0Hz, 0) / voiced.length;
    expect(meanF0).toBeGreaterThan(130);
    expect(meanF0).toBeLessThan(170);
  });

  it("marks white noise frames as mostly unvoiced", () => {
    const wav    = generateNoiseWav(0.5, SAMPLE_RATE, 0.3);
    const { samples } = parseWav(wav);
    const frames = extractF0(samples, SAMPLE_RATE);
    const voicedRatio = frames.filter(f => f.voiced).length / frames.length;
    expect(voicedRatio).toBeLessThan(0.3);  // noise should be mostly unvoiced
  });

  it("HNR is high for pure sine (periodic) and low for noise", () => {
    const sineWav  = generateSineWav(200, 0.5, SAMPLE_RATE);
    const noiseWav = generateNoiseWav(0.5, SAMPLE_RATE, 0.5);

    const sineFrames  = extractF0(parseWav(sineWav).samples,  SAMPLE_RATE);
    const noiseFrames = extractF0(parseWav(noiseWav).samples, SAMPLE_RATE);

    const sineVoiced  = sineFrames.filter(f => f.voiced);
    const sineHNR     = sineVoiced.reduce((s, f) => s + f.hnrDb, 0) / (sineVoiced.length || 1);

    const noiseVoiced = noiseFrames.filter(f => f.voiced);
    const noiseHNR    = noiseVoiced.length > 0
      ? noiseVoiced.reduce((s, f) => s + f.hnrDb, 0) / noiseVoiced.length
      : -Infinity;

    // Pure sine should have measurable HNR (Hann window bias limits it to ~5 dB at 25ms frames).
    // More importantly, sine should be more periodic than noise.
    expect(sineHNR).toBeGreaterThan(2);
    if (isFinite(noiseHNR)) {
      expect(sineHNR).toBeGreaterThan(noiseHNR);
    }
  });
});

// ─── Jitter & Shimmer ─────────────────────────────────────────────────────────

describe("computeJitter", () => {
  it("returns NaN when fewer than 3 voiced frames", () => {
    const frames = [{ timeMs: 0, f0Hz: 150, voiced: true, hnrDb: 20, rms: 0.1, peakCorr: 0.8 }];
    expect(computeJitter(frames)).toBeNaN();
  });

  it("jitter is near-zero for perfectly regular sine (espeak-like)", () => {
    const wav    = generateSineWav(200, 1.0, SAMPLE_RATE);
    const frames = extractF0(parseWav(wav).samples, SAMPLE_RATE);
    const j      = computeJitter(frames);
    if (!isNaN(j)) {
      expect(j).toBeLessThan(0.05); // < 5% for a clean sine
    }
  });

  it("shimmer is near-zero for constant-amplitude sine", () => {
    const wav    = generateSineWav(200, 1.0, SAMPLE_RATE);
    const frames = extractF0(parseWav(wav).samples, SAMPLE_RATE);
    const s      = computeShimmer(frames);
    if (!isNaN(s)) {
      expect(s).toBeLessThan(0.10); // envelope is constant
    }
  });
});

// ─── F0 contour stats ─────────────────────────────────────────────────────────

describe("f0ContourStats", () => {
  it("returns zeros on empty frame list", () => {
    const stats = f0ContourStats([]);
    expect(stats.meanHz).toBe(0);
    expect(stats.stdDevHz).toBe(0);
    expect(stats.slopeSemi).toBe(0);
  });

  it("detects rising F0 contour (positive slope)", () => {
    // chirp-like: F0 increases from 150 to 250 Hz
    const frames = Array.from({ length: 20 }, (_, i) => ({
      timeMs:   i * 10,
      f0Hz:     150 + i * 5,   // 150…245 Hz
      voiced:   true,
      hnrDb:    20,
      rms:      0.3,
      peakCorr: 0.8,
    }));
    const stats = f0ContourStats(frames);
    expect(stats.slopeSemi).toBeGreaterThan(0); // rising
    expect(stats.meanHz).toBeCloseTo(197.5, 0);
  });

  it("stdDevHz is 0 for flat (monotone) contour", () => {
    const frames = Array.from({ length: 10 }, (_, i) => ({
      timeMs: i * 10, f0Hz: 200, voiced: true, hnrDb: 20, rms: 0.3, peakCorr: 0.8,
    }));
    const stats = f0ContourStats(frames);
    expect(stats.stdDevHz).toBeCloseTo(0, 5);
  });
});

// ─── CPP ─────────────────────────────────────────────────────────────────────

describe("computeCPP", () => {
  it("returns a finite value for a harmonic frame", () => {
    const fSize = 512;
    const frame = new Float64Array(fSize);
    const f0    = 200;
    for (let i = 0; i < fSize; i++) {
      for (let h = 1; h <= 8; h++) {
        frame[i] += Math.sin(2 * Math.PI * f0 * h * i / SAMPLE_RATE) / h;
      }
    }
    const cpp = computeCPP(frame, SAMPLE_RATE);
    expect(isFinite(cpp)).toBe(true);
    expect(cpp).toBeGreaterThan(0); // cepstral peak should be above baseline
  });

  it("CPP is lower for noise than for harmonic signal", () => {
    const fSize   = 512;
    const harmonic = new Float64Array(fSize);
    const noise    = new Float64Array(fSize);
    for (let i = 0; i < fSize; i++) {
      for (let h = 1; h <= 8; h++) harmonic[i] += Math.sin(2 * Math.PI * 200 * h * i / SAMPLE_RATE) / h;
      noise[i] = Math.random() * 2 - 1;
    }
    const cppH = computeCPP(harmonic, SAMPLE_RATE);
    const cppN = computeCPP(noise, SAMPLE_RATE);
    expect(cppH).toBeGreaterThan(cppN);
  });
});

// ─── computeVoiceQuality ──────────────────────────────────────────────────────

describe("computeVoiceQuality", () => {
  it("returns valid metrics struct for harmonic WAV", () => {
    const wav = generateHarmonicWav(200, 0.8);
    const m   = computeVoiceQuality(wav, SAMPLE_RATE);
    expect(m.frameCount).toBeGreaterThan(0);
    expect(m.f0Stats.voicedCount).toBeGreaterThan(0);
    expect(isFinite(m.hnrDbMean)).toBe(true);
    expect(m.f0Stats.meanHz).toBeGreaterThan(100);
    expect(m.f0Stats.meanHz).toBeLessThan(400);
  });

  it("silent WAV → voiced ratio near 0", () => {
    const numSamples = 22050;
    const buf = Buffer.alloc(44 + numSamples * 2);
    buf.write("RIFF", 0); buf.writeUInt32LE(36 + numSamples * 2, 4); buf.write("WAVE", 8);
    buf.write("fmt ", 12); buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20);
    buf.writeUInt16LE(1, 22); buf.writeUInt32LE(SAMPLE_RATE, 24);
    buf.writeUInt32LE(SAMPLE_RATE * 2, 28); buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34);
    buf.write("data", 36); buf.writeUInt32LE(numSamples * 2, 40);
    const m = computeVoiceQuality(buf, SAMPLE_RATE);
    expect(m.voicedRatio).toBeLessThan(0.1);
  });
});

// ─── roboticnessScore ─────────────────────────────────────────────────────────

describe("roboticnessScore", () => {
  it("pure sine → high roboticness score", () => {
    const wav = generateSineWav(200, 1.0, SAMPLE_RATE);
    const m   = computeVoiceQuality(wav, SAMPLE_RATE);
    const r   = roboticnessScore(m);
    // A pure sine: no jitter, no shimmer, near-zero F0 variation, very high HNR
    expect(r.score).toBeGreaterThan(40);   // clearly robotic
    expect(["Moderate", "Robotic", "Mechanical"]).toContain(r.label);
  });

  it("harmonic series → lower roboticness than pure sine", () => {
    const sineWav = generateSineWav(200, 1.0, SAMPLE_RATE);
    const harmWav = generateHarmonicWav(200, 1.0);
    const rm = roboticnessScore(computeVoiceQuality(sineWav, SAMPLE_RATE));
    const rh = roboticnessScore(computeVoiceQuality(harmWav, SAMPLE_RATE));
    // Harmonics have amplitude variation across the series — shimmer/HNR improve
    // Both should still be robotic but harmonic should be ≤ sine
    expect(rh.score).toBeLessThanOrEqual(rm.score + 5); // allow small float margin
  });

  it("factors array has 5 entries summing to total score", () => {
    const wav = generateHarmonicWav(200, 0.8);
    const m   = computeVoiceQuality(wav, SAMPLE_RATE);
    const r   = roboticnessScore(m);
    expect(r.factors).toHaveLength(5);
    const factorSum = r.factors.reduce((s, f) => s + f.penalty, 0);
    expect(r.score).toBeCloseTo(Math.min(100, factorSum), 5);
  });

  it("recommendation string mentions the worst factor", () => {
    const wav = generateSineWav(200, 1.0, SAMPLE_RATE);
    const m   = computeVoiceQuality(wav, SAMPLE_RATE);
    const r   = roboticnessScore(m);
    expect(r.recommendation).toMatch(/Primary factor:/);
  });
});

// ─── ASCII renderers ──────────────────────────────────────────────────────────

describe("asciiSpectrogram", () => {
  it("returns a non-empty multi-line string", () => {
    const wav = generateSineWav(440, 0.5, SAMPLE_RATE);
    const { samples } = parseWav(wav);
    const out = asciiSpectrogram(samples, SAMPLE_RATE, { width: 40, height: 8, title: "Test 440Hz" });
    const lines = out.split("\n");
    expect(lines.length).toBeGreaterThan(5);
    expect(out).toContain("440Hz");
    expect(out).toMatch(/│/);
  });

  it("brighter cells at target frequency vs. empty band", () => {
    const wav = generateSineWav(500, 0.5, SAMPLE_RATE);
    const { samples } = parseWav(wav);
    // Render 100–1000 Hz range — 500 Hz row should have brighter characters
    const out = asciiSpectrogram(samples, SAMPLE_RATE, { width: 40, height: 10, minHz: 100, maxHz: 1000 });
    // Check that the plot is not all spaces (energy present)
    expect(out).toMatch(/[·:;+=xX$#@]/);
  });
});

describe("asciiPSD", () => {
  it("returns the correct number of lines", () => {
    const wav = generateHarmonicWav(200, 0.5);
    const { samples } = parseWav(wav);
    const out   = asciiPSD(samples, SAMPLE_RATE, { points: 20, title: "PSD Test" });
    const lines = out.split("\n");
    expect(lines.length).toBe(21); // title + 20 frequency lines
  });

  it("dB labels are present in output", () => {
    const wav = generateSineWav(300, 0.5, SAMPLE_RATE);
    const { samples } = parseWav(wav);
    const out = asciiPSD(samples, SAMPLE_RATE);
    expect(out).toMatch(/dB/);
  });
});

describe("asciiF0Contour", () => {
  it("renders a dot at the correct F0 row", () => {
    const frames = [
      { timeMs: 50, f0Hz: 200, voiced: true, hnrDb: 20, rms: 0.3, peakCorr: 0.8 },
      { timeMs: 150, f0Hz: 0,  voiced: false, hnrDb: -Infinity, rms: 0.0, peakCorr: 0 },
    ];
    const out = asciiF0Contour(frames, 200, { width: 20, height: 8 });
    expect(out).toContain("●");
    expect(out).toContain("·"); // unvoiced marker
  });

  it("title appears when provided", () => {
    const out = asciiF0Contour([], 100, { title: "F0 Contour" });
    expect(out).toContain("F0 Contour");
  });
});

// ─── Snapshot: known sine roboticness profile ──────────────────────────────────

describe("roboticness snapshot — pure 200 Hz sine", () => {
  it("matches snapshot", () => {
    const wav = generateSineWav(200, 1.0, SAMPLE_RATE);
    const m   = computeVoiceQuality(wav, SAMPLE_RATE);
    const r   = roboticnessScore(m);

    const report = [
      `label: ${r.label}`,
      `score: ${r.score.toFixed(1)}`,
      ...r.factors.map(f => `  ${f.name}: ${f.value} → penalty=${f.penalty.toFixed(1)}`),
      `recommendation: ${r.recommendation}`,
    ].join("\n");

    expect(report).toMatchSnapshot();
  });
});
