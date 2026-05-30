/**
 * Translation chunking test harness
 *
 * Measures the tradeoff between chunking granularity and translation quality.
 * When an LLM translator sees shorter chunks it loses inter-sentence context,
 * producing worse output. Smaller chunks = more cache keys = higher hit rate.
 *
 * Three levels tested per paragraph:
 *   paragraph  — 1 translate call, 1 cache key, full context
 *   sentence   — N calls (split on .!?), N keys, sentence context only
 *   clause     — M calls (split on ,;), M keys, clause context only
 *
 * Run (silent — shows quality table only):
 *   npx vitest run translation-chunking
 *
 * Run with a live translator (LibreTranslate on :5000):
 *   TRANSLATE=1 npx vitest run translation-chunking
 */

import { describe, it, expect, beforeAll } from "vitest";
import {
  makeITGlossaryMiddleware,
  makeTranslateMiddleware,
  PipelineTranslator,
  IdentityTranslator,
} from "./pipeline/translators.ts";

const TRANSLATE = process.env.TRANSLATE === "1";

// ─── Test corpus ─────────────────────────────────────────────────────────────
// IT/engineering paragraphs with hand-written expected Russian output.
// Expected output uses natural Russian developer speech.

const CORPUS = [
  {
    id: "deploy-commit",
    en: "Deploy the service to production. Commit the hotfix and push to the branch. Run the test suite before merging.",
    expectedRu: "Задеплойте сервис на прод. Закоммитьте хотфикс и запушьте в ветку. Запустите тесты перед мержем.",
    keywords: ["прод", "ветку", "тест"],
  },
  {
    id: "debug-pipeline",
    en: "Debug the failing pipeline. Check the logs and fix the build. The frontend cache needs to be invalidated after the deploy.",
    expectedRu: "Дебажьте упавший пайплайн. Проверьте логи и почините билд. Кэш фронтенда нужно инвалидировать после деплоя.",
    keywords: ["пайплайн", "логи", "билд", "кэш", "фронтенда", "деплоя"],
  },
  {
    id: "review-refactor",
    en: "Review the pull request before the sprint ends. Refactor the backend API to reduce latency. The database query needs an index.",
    expectedRu: "Сделайте ревью пуллреквеста до конца спринта. Отрефакторьте бэкенд API для снижения латентности. Запросу к базе данных нужен индекс.",
    keywords: ["пуллреквеста", "спринта", "бэкенд", "API"],
  },
  {
    id: "standup",
    en: "Yesterday I fixed the bug in the cache layer. Today I will merge the feature branch and deploy to staging. Blocked on the code review.",
    expectedRu: "Вчера починил баг в слое кэша. Сегодня смержу ветку с фичей и задеплою на стейджинг. Заблокирован на ревью кода.",
    keywords: ["кэша", "ветку", "стейджинг", "ревью"],
  },
] as const;

// ─── Chunkers ────────────────────────────────────────────────────────────────

function chunkParagraph(text: string): string[] {
  return [text.trim()];
}

function chunkSentence(text: string): string[] {
  return text.split(/(?<=[.!?])\s+/).filter(s => s.trim().length > 2);
}

function chunkClause(text: string): string[] {
  return text
    .split(/(?<=[.!?,;])\s+/)
    .map(s => s.trim())
    .filter(s => s.length > 2);
}

const CHUNKERS = {
  paragraph: chunkParagraph,
  sentence:  chunkSentence,
  clause:    chunkClause,
} as const;

// ─── Quality measurement ──────────────────────────────────────────────────────

/**
 * Keyword hit rate — how many expected Russian IT terms appear in actual output.
 * Simple but meaningful: if the translator is producing the right IT loanwords
 * the keywords will be present.
 */
function keywordHitRate(actual: string, keywords: readonly string[]): number {
  const lower = actual.toLowerCase();
  const hits = keywords.filter(k => lower.includes(k.toLowerCase()));
  return hits.length / keywords.length;
}

/**
 * Token overlap — rough similarity between expected and actual.
 * Splits on whitespace/punctuation, counts shared tokens.
 */
function tokenOverlap(expected: string, actual: string): number {
  const tokenise = (s: string) => new Set(
    s.toLowerCase().split(/[\s.,!?;:—]+/).filter(Boolean)
  );
  const e = tokenise(expected);
  const a = tokenise(actual);
  let hits = 0;
  for (const t of e) if (a.has(t)) hits++;
  return hits / Math.max(e.size, 1);
}

// ─── Translator ───────────────────────────────────────────────────────────────

async function translate(chunks: string[]): Promise<string[]> {
  if (!TRANSLATE) {
    // Without a live server: apply IT glossary only (demonstrates the pre-processing)
    const mw  = makeITGlossaryMiddleware();
    return Promise.all(chunks.map(async chunk => {
      const ctx = { input: chunk, text: chunk, lang: "en" as const };
      await mw(ctx, async () => {});
      return ctx.text;
    }));
  }

  // With LibreTranslate running on :5000
  const t = new PipelineTranslator(
    [makeITGlossaryMiddleware(), makeTranslateMiddleware("en", "ru")],
    "ru",
  );
  return Promise.all(chunks.map(c => t.translate(c)));
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("Translation chunking: quality vs cache granularity", () => {
  const results: Array<{
    id:          string;
    level:       string;
    chunks:      number;
    cacheKeys:   number;
    keywordRate: number;
    tokenSim:    number;
    actual:      string;
  }> = [];

  beforeAll(async () => {
    if (!TRANSLATE) {
      console.info("\n[chunking] Running in glossary-only mode. Set TRANSLATE=1 for live LibreTranslate.");
    }
  });

  for (const { id, en, expectedRu, keywords } of CORPUS) {
    for (const [level, chunker] of Object.entries(CHUNKERS) as [string, (t: string) => string[]][]) {
      it(`[${id}] ${level}`, async () => {
        const chunks    = chunker(en);
        const translated = await translate(chunks);
        const actual    = translated.join(" ");

        const keywordRate = keywordHitRate(actual, keywords);
        const tokenSim    = tokenOverlap(expectedRu, actual);

        results.push({
          id, level,
          chunks:      chunks.length,
          cacheKeys:   chunks.length,  // one cache key per chunk
          keywordRate,
          tokenSim,
          actual,
        });

        // Print inline for visibility
        console.info(`\n  [${id}/${level}] chunks=${chunks.length}  keywords=${(keywordRate * 100).toFixed(0)}%  similarity=${(tokenSim * 100).toFixed(0)}%`);
        if (TRANSLATE) console.info(`  actual: "${actual.slice(0, 80)}..."`);

        // Hard assert: glossary must have fired (IT terms present even without full translation)
        const lower = actual.toLowerCase();
        const hasAnyITTerm = ["деплой", "коммит", "мерж", "пайплайн", "кэш", "билд",
                              "бэкенд", "фронтенд", "тест", "баг", "ревью", "спринт",
                              "стейджинг", "пуллреквест", "ветк"].some(t => lower.includes(t));
        expect(hasAnyITTerm).toBe(true);
      });
    }
  }

  it("summary table: quality vs cache keys per level", async () => {
    if (results.length === 0) return; // tests haven't run yet in this context

    const byLevel: Record<string, { keywordRate: number[]; tokenSim: number[]; cacheKeys: number[] }> = {};
    for (const r of results) {
      byLevel[r.level] ??= { keywordRate: [], tokenSim: [], cacheKeys: [] };
      byLevel[r.level].keywordRate.push(r.keywordRate);
      byLevel[r.level].tokenSim.push(r.tokenSim);
      byLevel[r.level].cacheKeys.push(r.cacheKeys);
    }

    const avg = (arr: number[]) => arr.reduce((a, b) => a + b, 0) / arr.length;

    console.info("\n┌────────────┬──────────┬────────────┬──────────────┐");
    console.info("│ Level      │ Avg keys │ Keywords % │ Token sim %  │");
    console.info("├────────────┼──────────┼────────────┼──────────────┤");
    for (const [level, data] of Object.entries(byLevel)) {
      const keys = avg(data.cacheKeys).toFixed(1);
      const kw   = (avg(data.keywordRate) * 100).toFixed(0);
      const sim  = (avg(data.tokenSim) * 100).toFixed(0);
      console.info(`│ ${level.padEnd(10)} │ ${keys.padStart(8)} │ ${(kw + "%").padStart(10)} │ ${(sim + "%").padStart(12)} │`);
    }
    console.info("└────────────┴──────────┴────────────┴──────────────┘");
    console.info("\n  Higher keys = better cache hit rate");
    console.info("  Higher keywords/similarity = better translation quality");
    console.info("  Golden middle: sentence level (balance of both)");
  });
});
