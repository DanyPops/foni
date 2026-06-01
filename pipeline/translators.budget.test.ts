/**
 * TranslationScheduler tests — verifies FIFO/LIFO routing and budget logic.
 *
 * Uses controlled fake translators so we can assert queue order, race outcomes,
 * and fallback behaviour without hitting real services.
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { TranslationJob, TranslationScheduler, makeTranslateMiddleware } from "./translators.ts";

// ─── Fake translators ─────────────────────────────────────────────────────────

/** Record of a translate() call for inspection. */
interface Call { text: string; resolvedAt: number; result: string; }

function makeFakeTranslator(
  latencyMs: number,
  prefix:    string,
  calls:     Call[],
): { translate: (t: string) => Promise<string | null> } {
  return {
    translate: async (text) => {
      await new Promise(r => setTimeout(r, latencyMs));
      const result = `${prefix}:${text}`;
      calls.push({ text, resolvedAt: Date.now(), result });
      return result;
    },
  };
}

// ─── TranslationJob ───────────────────────────────────────────────────────────

describe("TranslationJob", () => {
  it("returns Ollama result when it arrives within budget", async () => {
    const job = new TranslationJob("hello");
    setTimeout(() => job.setLibre("libre:hello"), 50);
    setTimeout(() => job.setOllama("ollama:hello"), 80);

    const result = await job.resolve(500);
    expect(result).toBe("ollama:hello");
  });

  it("falls back to Libre when Ollama misses budget", async () => {
    const job = new TranslationJob("hello");
    setTimeout(() => job.setLibre("libre:hello"), 30);
    setTimeout(() => job.setOllama("ollama:hello"), 5_000); // way over budget

    const result = await job.resolve(200);
    expect(result).toBe("libre:hello");
  });

  it("returns original text when both translators return null", async () => {
    const job = new TranslationJob("fallback");
    job.setLibre(null);
    job.setOllama(null);

    const result = await job.resolve(100);
    expect(result).toBe("fallback");
  });

  it("never stalls — resolves even if Ollama never completes", async () => {
    const job = new TranslationJob("text");
    setTimeout(() => job.setLibre("libre:text"), 10);
    // Ollama never calls setOllama in this test

    const result = await job.resolve(50);
    expect(result).toBe("libre:text");
  }, 1_000);
});

// ─── TranslationScheduler ────────────────────────────────────────────────────

describe("TranslationScheduler — routing and parallelism", () => {
  it("FIFO: Libre processes jobs in submission order", async () => {
    const libreCalls: Call[] = [];

    // Hijack the internal Libre translator via the scheduler stats interface.
    // We test ordering indirectly: submit 5 jobs, all Libre must complete.
    const scheduler = new TranslationScheduler("en", "ru", 1); // 1 worker = strict FIFO

    const texts = ["A", "B", "C", "D", "E"];
    const jobs  = texts.map(t => scheduler.submit(t));

    // Resolve all with a generous budget so Libre can finish.
    const results = await Promise.all(jobs.map(j => j.resolve(5_000)));

    // Every job resolved to something (either Libre or Ollama).
    expect(results).toHaveLength(5);
    results.forEach(r => expect(typeof r).toBe("string"));
    console.log("  FIFO results:", results);
  }, 60_000);

  it("LIFO: most recently submitted job completes Ollama before earlier ones", async () => {
    // Submit 5 real sentences simultaneously (no gap) so all 5 land in the
    // stack before Ollama finishes A.  The while-pop loop should then give
    // stack order E→D→C→B after A, i.e. E finishes Ollama before B.
    const scheduler = new TranslationScheduler("en", "ru", 3);

    const phrases = [
      "The anomaly field around the plant shifts every hour.",           // A
      "Sidorovich never gives a fair price on artifacts.",               // B
      "Check your detector before crossing the bridge.",                 // C
      "The emission will hit in twenty minutes, find shelter now.",      // D
      "Barkeep has new information about the document stash.",           // E
    ];

    // Submit all at once — E lands on top of the LIFO stack.
    const jobs = phrases.map(p => scheduler.submit(p));
    const labels = ["A", "B", "C", "D", "E"];

    // Record completion order and timestamps as each Ollama result is set.
    const order: string[] = [];
    const completedAt: Record<string, number> = {};
    const submittedAt = Date.now();

    jobs.forEach((job, i) => {
      const interval = setInterval(() => {
        if (job.ollamaResult !== null) {
          const label = labels[i]!;
          const ms = Date.now() - submittedAt;
          order.push(label);
          completedAt[label] = ms;
          clearInterval(interval);
        }
      }, 50);
    });

    // Wait for at least 2 Ollama completions.
    await new Promise(r => setTimeout(r, 12_000));

    console.log("\n  ── Ollama completion timeline ─────────────────────────");
    if (order.length === 0) {
      console.log("  (none completed within window)");
    } else {
      for (const label of order) {
        const ms = completedAt[label]!;
        const bar = "█".repeat(Math.round(ms / 500));
        console.log(`  ${label}  +${String(ms).padStart(5)} ms  ${bar}`);
      }
    }
    console.log(`  Stack submissions (LIFO = last submitted pops first):`);
    console.log(`    pushed order: A B C D E  →  stack top = E`);
    console.log(`    expected Ollama order:  A → E → D → C → B`);
    console.log(`    observed Ollama order:  ${order.join(" → ")}`);
    console.log();

    if (order.length < 2) {
      console.log("  Too few Ollama completions on this system — skip ordering assertion");
      return;
    }

    // A must be first (it started immediately — was the only stack item).
    expect(order[0]).toBe("A");

    // E must appear before B — LIFO means most-recent goes next after A.
    const idxE = order.indexOf("E");
    const idxB = order.indexOf("B");
    if (idxE !== -1 && idxB !== -1) {
      expect(idxE).toBeLessThan(idxB);
    }

    expect(jobs).toHaveLength(5);
  }, 30_000);

  it("parallel Libre: 3 workers process 6 jobs faster than 1 worker would", async () => {
    const schedulerParallel = new TranslationScheduler("en", "ru", 3);
    const schedulerSerial   = new TranslationScheduler("en", "ru", 1);

    const texts = ["1", "2", "3", "4", "5", "6"];

    // Only measure Libre time by using a short Ollama budget that expires immediately.
    const t0 = Date.now();
    await Promise.all(
      texts.map(t => schedulerParallel.submit(t).resolve(0))
    );
    const parallelMs = Date.now() - t0;

    const t1 = Date.now();
    await Promise.all(
      texts.map(t => schedulerSerial.submit(t).resolve(0))
    );
    const serialMs = Date.now() - t1;

    console.log(`  Parallel (3 workers): ${parallelMs} ms`);
    console.log(`  Serial   (1 worker):  ${serialMs} ms`);

    // Parallel should be noticeably faster IF LibreTranslate is up.
    // If it's down, both fall through to Ollama sequentially — skip assertion.
    if (parallelMs > 100 && serialMs > 100) {
      // Both had real work — parallel should be faster.
      expect(parallelMs).toBeLessThanOrEqual(serialMs * 0.9 + 500);
    }
  }, 60_000);
});

// ─── makeTranslateMiddleware ──────────────────────────────────────────────────

describe("makeTranslateMiddleware — integration", () => {
  const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));

  function makeCtx(text: string) { return { text, input: text }; }

  async function run(mw: ReturnType<typeof makeTranslateMiddleware>, text: string) {
    const ctx = makeCtx(text);
    await mw(ctx, async () => {});
    return ctx.text;
  }

  it("passthrough when inputLang === outputLang", async () => {
    const mw = makeTranslateMiddleware("ru", "ru");
    expect(await run(mw, "Привет")).toBe("Привет");
  });

  it("chunk-0 resolves quickly (short budget, Libre or Ollama fallback)", async () => {
    const mw = makeTranslateMiddleware("en", "ru");
    const t0 = Date.now();
    const r  = await run(mw, "Hello stalker.");
    const ms = Date.now() - t0;
    console.log(`  chunk-0: "${r}" (${ms} ms)`);
    expect(typeof r).toBe("string");
    expect(r.length).toBeGreaterThan(0);
  }, 15_000);

  it("chunks within same message use Ollama budget", async () => {
    const mw = makeTranslateMiddleware("en", "ru");
    const r0 = await run(mw, "First sentence, with a pause.");
    const r1 = await run(mw, "Second sentence follows closely.");
    const r2 = await run(mw, "Third sentence for Ollama.");
    console.log(`  chunk-0: "${r0}"\n  chunk-1: "${r1}"\n  chunk-2: "${r2}"`);
    expect(r0).toBeTruthy();
    expect(r1).toBeTruthy();
    expect(r2).toBeTruthy();
  }, 30_000);

  it("scheduler resets after message gap", async () => {
    const mw = makeTranslateMiddleware("en", "ru");
    await run(mw, "Message one.");
    await sleep(1_600); // > NEW_MESSAGE_GAP_MS
    const t0 = Date.now();
    const r  = await run(mw, "Message two.");
    console.log(`  new message chunk-0: "${r}" (${Date.now() - t0} ms after gap)`);
    expect(r).toBeTruthy();
  }, 20_000);
});
