import { describe, it, expect, beforeEach } from "vitest";
import { scorePlacement, WordDiversifier } from "./pipeline/translators.ts";

// ─── scorePlacement ───────────────────────────────────────────────────────────

describe("scorePlacement", () => {
  it("all scores are within [0, 1] for various sentence types", () => {
    const sentences = [
      "Привет.",
      "Деплой завершён, баг исправлен, пайплайн зелёный.",
      "Всё нормально?",
      "О.",
      "Это очень длинное предложение с большим количеством слов и смысла.",
    ];
    for (const s of sentences) {
      const { prefix, mid, suffix } = scorePlacement(s);
      expect(prefix, `prefix for: ${s}`).toBeGreaterThanOrEqual(0);
      expect(prefix, `prefix for: ${s}`).toBeLessThanOrEqual(1);
      expect(mid,    `mid for: ${s}`).toBeGreaterThanOrEqual(0);
      expect(mid,    `mid for: ${s}`).toBeLessThanOrEqual(1);
      expect(suffix, `suffix for: ${s}`).toBeGreaterThanOrEqual(0);
      expect(suffix, `suffix for: ${s}`).toBeLessThanOrEqual(1);
    }
  });

  it("question → suffix = 0 (don't append mat to questions)", () => {
    expect(scorePlacement("Всё нормально?").suffix).toBe(0);
    expect(scorePlacement("Что случилось?").suffix).toBe(0);
  });

  it("statement → suffix > 0", () => {
    expect(scorePlacement("Всё сломалось.").suffix).toBeGreaterThan(0);
  });

  it("long sentence boosts suffix above short sentence", () => {
    const short = scorePlacement("Ок.");
    const long  = scorePlacement("Деплой завершён и все тесты прошли успешно как мы и ожидали.");
    expect(long.suffix).toBeGreaterThan(short.suffix);
  });

  it("single-clause sentence → mid = 0 (no comma breaks)", () => {
    expect(scorePlacement("Понятно.").mid).toBe(0);
    expect(scorePlacement("Что случилось?").mid).toBe(0);
  });

  it("multi-clause sentence → mid > 0", () => {
    expect(scorePlacement("Первое, второе.").mid).toBeGreaterThan(0);
  });

  it("mid scales with clause count", () => {
    const one   = scorePlacement("Раз.");
    const two   = scorePlacement("Раз, два.");
    const three = scorePlacement("Раз, два, три.");
    expect(two.mid).toBeGreaterThan(one.mid);
    expect(three.mid).toBeGreaterThan(two.mid);
  });

  it("technical content boosts prefix", () => {
    const tech   = scorePlacement("Деплой упал в проде.");
    const notech = scorePlacement("Пора отдыхать.");
    expect(tech.prefix).toBeGreaterThan(notech.prefix);
  });

  it("baseline prefix > 0 even without tech content", () => {
    expect(scorePlacement("Пора отдыхать.").prefix).toBeGreaterThan(0);
  });
});

// ─── WordDiversifier ──────────────────────────────────────────────────────────

describe("WordDiversifier", () => {
  let d: WordDiversifier;
  beforeEach(() => { d = new WordDiversifier(); });

  it("returns items from the array", () => {
    const arr = ["а", "б", "в"];
    for (let i = 0; i < 30; i++) {
      expect(arr).toContain(d.pick(arr));
    }
  });

  it("covers all items over enough iterations", () => {
    const arr = ["а", "б", "в"];
    const seen = new Set<string>();
    for (let i = 0; i < 60; i++) seen.add(d.pick(arr));
    expect(seen.size).toBe(3);
  });

  it("increments getCount on each pick", () => {
    d.pick(["блядь"]);
    d.pick(["блядь"]);
    expect(d.getCount("блядь")).toBe(2);
  });

  it("reset() clears all counts", () => {
    d.pick(["блядь"]);
    d.pick(["блядь"]);
    d.reset();
    expect(d.getCount("блядь")).toBe(0);
  });

  it("handles single-item array", () => {
    expect(d.pick(["только"])).toBe("только");
    expect(d.getCount("только")).toBe(1);
  });

  it("throws on empty array", () => {
    expect(() => d.pick([])).toThrow("empty array");
  });

  it("down-weights overused words: fresh word wins the first N picks", () => {
    // Pre-use "а" 10 times — weight_а = 1/11 ≈ 0.09, weight_б = 1/1
    // P(б on first pick) ≈ 0.917; sample only the first 30 picks where
    // the advantage is largest before counts converge.
    for (let i = 0; i < 10; i++) d.pick(["а"]);
    expect(d.getCount("а")).toBe(10);

    let bCount = 0;
    for (let i = 0; i < 30; i++) {
      if (d.pick(["а", "б"]) === "б") bCount++;
    }
    // б starts with zero count while а has 10 — so б must win most picks.
    expect(bCount).toBeGreaterThan(12);
  });

  it("counts are independent per instance", () => {
    const d2 = new WordDiversifier();
    d.pick(["блядь"]);
    expect(d.getCount("блядь")).toBe(1);
    expect(d2.getCount("блядь")).toBe(0);
  });
});
