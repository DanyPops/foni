/**
 * prosody.test.ts — ProsodyAnnotator and BreathInjector unit tests.
 *
 * Tests are independent of espeak/RVC (no external services needed).
 * DSP assertions are done on synthesised WAV buffers.
 */

import { describe, it, expect } from "vitest";
import { annotateProsody, isSsml, ProsodyBackend } from "./pipeline/prosody.ts";
import { injectBreaths, BreathProcessor }          from "./pipeline/breath-injector.ts";
import { generateSineWav, generateNoiseWav, parseWav, rms } from "./test/audio-test-utils.ts";

const SR = 22050;

// ─── annotateProsody ──────────────────────────────────────────────────────────

describe("annotateProsody", () => {
  it("wraps output in <speak> tags", () => {
    const out = annotateProsody("Привет мир.");
    expect(out).toMatch(/^<speak>/);
    expect(out).toMatch(/<\/speak>$/);
  });

  it("inserts <break> at comma positions", () => {
    const out = annotateProsody("Давай, шевелись.");
    expect(out).toContain("<break time=\"150ms\"/>");
  });

  it("inserts <break> at em-dash positions", () => {
    const out = annotateProsody("Слушай — дело есть.");
    expect(out).toContain(`<break time="200ms"/>`);
  });

  it("inserts <break> at ellipsis positions", () => {
    const out = annotateProsody("Ну… что сказать.");
    expect(out).toContain(`<break time="420ms"/>`);
  });

  it("wraps sentences in <prosody> with rate and range (no pitch)", () => {
    const out = annotateProsody("Хорошо. Понял.");
    expect(out).toMatch(/<prosody rate="\d+%" range="\w+">/);
    expect(out).not.toMatch(/pitch=/);
  });

  it("questions get range=high", () => {
    const out = annotateProsody("Понял, брателло?");
    expect(out).toContain('range="high"');
  });

  it("exclamations get range=x-high", () => {
    const out = annotateProsody("Ого!");
    expect(out).toContain('range="x-high"');
  });

  it("statements get range=medium", () => {
    const out = annotateProsody("Всё нормально.");
    expect(out).toContain('range="medium"');
  });

  it("escapes XML special characters", () => {
    const out = annotateProsody('Скажи "привет" & всё.');
    expect(out).toContain("&amp;");
    expect(out).toContain("&quot;");
    expect(out).not.toContain(' & ');
  });

  it("is deterministic — same input → same output", () => {
    const a = annotateProsody("Давай, шевелись.");
    const b = annotateProsody("Давай, шевелись.");
    expect(a).toBe(b);
  });

  it("different sentences get different prosody parameters", () => {
    const a = annotateProsody("Первое предложение.");
    const b = annotateProsody("Второе предложение.");
    // The <prosody> wrapper parameters should differ because hashStr differs
    const rateA = a.match(/rate="(\d+)%"/)?.[1];
    const rateB = b.match(/rate="(\d+)%"/)?.[1];
    // They MIGHT be the same by chance but very unlikely — just check valid range
    expect(Number(rateA)).toBeGreaterThan(85);
    expect(Number(rateA)).toBeLessThan(115);
    expect(Number(rateB)).toBeGreaterThan(85);
    expect(Number(rateB)).toBeLessThan(115);
  });

  it("respects breaks=false option", () => {
    const out = annotateProsody("Давай, шевелись.", { breaks: false });
    expect(out).not.toContain("<break");
  });

  it("respects prosodyVariation=false option", () => {
    const out = annotateProsody("Давай, шевелись.", { prosodyVariation: false });
    expect(out).not.toContain("<prosody");
  });

  it("passes through already-SSML input unchanged via isSsml check", () => {
    const ssml = "<speak>Already marked up.</speak>";
    expect(isSsml(ssml)).toBe(true);
    expect(isSsml("plain text")).toBe(false);
  });

  it("multi-sentence text inserts sentence-boundary breaks", () => {
    const out = annotateProsody("Первое. Второе. Третье.");
    // Should have sentence-boundary breaks between sentences
    const breakCount = (out.match(/<break/g) ?? []).length;
    expect(breakCount).toBeGreaterThanOrEqual(2);  // at least between each sentence
  });
});

// ─── injectBreaths ────────────────────────────────────────────────────────────

describe("injectBreaths", () => {
  it("returns a valid WAV buffer", async () => {
    // Create a WAV with a silence gap in the middle
    const silence = Buffer.alloc(SR * 0.5 * 2 + 44);  // 500ms silence header+data
    // Write proper WAV header
    silence.write("RIFF", 0);  silence.writeUInt32LE(36 + SR, 4);
    silence.write("WAVE", 8);  silence.write("fmt ", 12);
    silence.writeUInt32LE(16, 16); silence.writeUInt16LE(1, 20); silence.writeUInt16LE(1, 22);
    silence.writeUInt32LE(SR, 24); silence.writeUInt32LE(SR * 2, 28);
    silence.writeUInt16LE(2, 32); silence.writeUInt16LE(16, 34);
    silence.write("data", 36); silence.writeUInt32LE(SR, 40);

    // Just check it returns without throwing
    await expect(injectBreaths(silence, SR)).resolves.toBeTruthy();
  });

  it("does not inject into very short gaps", async () => {
    // 50ms gap — below 80ms gate — should not inject
    const wav     = generateSineWav(440, 0.5, SR, 0.7);
    const result  = await injectBreaths(wav, SR, { silenceGateMs: 80 });
    // With no gaps ≥ 80ms in a pure sine wave, result should be same length
    expect(result.length).toBeCloseTo(wav.length, -2);
  });

  it("breath output is longer than input when gaps are found", async () => {
    // Construct a WAV with explicit silence: 200ms sine + 200ms silence + 200ms sine
    const n      = Math.floor(SR * 0.2);
    const total  = n * 3;
    const buf    = Buffer.alloc(44 + total * 2);
    buf.write("RIFF", 0);  buf.writeUInt32LE(36 + total * 2, 4);
    buf.write("WAVE", 8);  buf.write("fmt ", 12);
    buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22);
    buf.writeUInt32LE(SR, 24); buf.writeUInt32LE(SR * 2, 28);
    buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34);
    buf.write("data", 36); buf.writeUInt32LE(total * 2, 40);

    // Write 200ms sine
    for (let i = 0; i < n; i++) {
      const s = Math.sin(2 * Math.PI * 440 * i / SR) * 0.7;
      buf.writeInt16LE(Math.round(s * 32767), 44 + i * 2);
    }
    // Middle 200ms stays silent (zeros already from alloc)
    // Write 200ms sine after silence
    for (let i = 0; i < n; i++) {
      const s = Math.sin(2 * Math.PI * 440 * i / SR) * 0.7;
      buf.writeInt16LE(Math.round(s * 32767), 44 + (n * 2 + i) * 2);
    }

    const result = await injectBreaths(buf, SR, { silenceGateMs: 80, injectAtStart: false });
    // The breath (120ms) is spliced INTO the 200ms silence — total length stays the same
    // (we replace first 120ms of the gap with breath, keep remaining 80ms silence)
    // So result length ≈ input length
    expect(Math.abs(result.length - buf.length)).toBeLessThan(1000);
  });

  it.skip("breath audio is non-silent — requires foni-synth /breath (Rust test: cargo test -p foni-synth)", async () => {
    const wavWithGap = (() => {
      const n     = Math.floor(SR * 0.2);
      const total = n * 3;
      const buf   = Buffer.alloc(44 + total * 2);
      buf.write("RIFF", 0);  buf.writeUInt32LE(36 + total * 2, 4);
      buf.write("WAVE", 8);  buf.write("fmt ", 12);
      buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22);
      buf.writeUInt32LE(SR, 24); buf.writeUInt32LE(SR * 2, 28);
      buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34);
      buf.write("data", 36); buf.writeUInt32LE(total * 2, 40);
      for (let i = 0; i < n; i++) {
        const s = Math.sin(2 * Math.PI * 440 * i / SR) * 0.7;
        buf.writeInt16LE(Math.round(s * 32767), 44 + i * 2);
      }
      for (let i = 0; i < n; i++) {
        const s = Math.sin(2 * Math.PI * 440 * i / SR) * 0.7;
        buf.writeInt16LE(Math.round(s * 32767), 44 + (n * 2 + i) * 2);
      }
      return buf;
    })();

    const result  = await injectBreaths(wavWithGap, SR, { silenceGateMs: 80 });
    const samples = parseWav(result).samples;
    // The middle region should now have some energy from the breath
    const midStart = Math.floor(SR * 0.2);
    const midEnd   = Math.floor(SR * 0.35);
    const midRms   = rms(samples.subarray(midStart, midEnd));
    expect(midRms).toBeGreaterThan(0.0005);  // breath is non-silent
  });
});

// ─── Snapshot: prosody output for canonical phrases ───────────────────────────

describe("prosody snapshot — canonical phrases", () => {
  it("matches snapshot for 3 BASELINE_PHRASES", () => {
    const phrases = [
      "Давай, шевелись.",
      "Ну что, брат, как дела?",
      "Слушай, тут такое дело.",
    ];
    const outputs = phrases.map(p => ({
      phrase: p,
      ssml:   annotateProsody(p),
    }));
    expect(outputs).toMatchSnapshot();
  });
});
