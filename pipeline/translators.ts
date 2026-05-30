import type { Translator } from "./interfaces.ts";
import type { WordBias, BiasWordMap } from "./interfaces.ts";

// ─── Translator defaults ────────────────────────────────────────────────────────────────

/** Probability of injecting a mat expression at each natural pause point. */
export const DEFAULT_MAT_PROBABILITY        = 0.35;

/** Probability that any injected mat word gets expressive vowel lengthening. */
export const DEFAULT_MAT_STRETCH_PROBABILITY = 0.5;

/** Probability of injecting an interjection at each natural pause point. */
export const DEFAULT_INTERJECT_PROBABILITY  = 0.25;

/** Minimum and maximum vowel repetitions during expressive lengthening. */
export const STRETCH_REPEATS_MIN = 2;
export const STRETCH_REPEATS_MAX = 4;

/** HTTP timeout for translation API calls. */
export const TRANSLATION_TIMEOUT_MS = 5_000;

// ─── Placement scoring ─────────────────────────────────────────────────────────────────
//
// Research: Russian mat fillers (блядь, сука) are most commonly placed at
// sentence end as general emphasizers. Mid-clause suits longer multi-clause
// sentences. Prefix fronting is for strong emphasis and is rarer.
// Source: Reddit/linguistic observation — "сука блять is most commonly placed
// at the end of a sentence as a general emphasizer."

export interface PlacementScores {
  /** 0–1 score for sentence-prefix injection. */
  prefix: number;
  /** 0–1 score for mid-clause injection (between comma breaks). */
  mid: number;
  /** 0–1 score for sentence-suffix injection. */
  suffix: number;
}

/**
 * Score a sentence for natural mat/interject injection placement.
 *
 * Factors:
 *  - Clause count (commas+1) → scales mid score
 *  - Word count             → longer sentence boosts suffix
 *  - IT/technical content   → boosts prefix (frustration emphasis)
 *  - Question mark          → suppresses suffix entirely
 */
export function scorePlacement(sentence: string): PlacementScores {
  if (!sentence) return { prefix: 0, mid: 0, suffix: 0 };
  const clauseCount = (sentence.match(/,/g) ?? []).length + 1;
  const wordCount   = sentence.trim().split(/\s+/).length;
  const isQuestion  = /\?$/.test(sentence.trim());
  const hasTech     = /(?<![а-яёА-ЯЁa-z0-9_])(баг|деплой|пайплайн|коммит|API|HTTP|сервер|лог|тест|билд|фича|фикс|кэш|бэкенд|фронтенд)(?![а-яёА-ЯЁa-z0-9_])/i.test(sentence);

  // Suffix: natural home for fillers; suppress on questions; long sentences boost it
  const suffix = isQuestion ? 0 : Math.min(1, 0.55 + (wordCount > 8 ? 0.2 : 0));

  // Mid: needs clause breaks to sound natural; scales with clause count
  const mid = Math.min(1, (clauseCount - 1) * 0.4);

  // Prefix: rarer; technical frustration and long sentences push it up
  const prefix = Math.min(1, 0.25 + (hasTech ? 0.2 : 0) + (wordCount > 12 ? 0.1 : 0));

  return { prefix, mid, suffix };
}

// ─── Word Diversifier ──────────────────────────────────────────────────────────────────
//
// Prevents repetition: if "блядь" was just injected, the next pick favours
// other words. Weight = 1 / (1 + usageCount) — halves probability each use.
// One instance per middleware closure, persists across TTS calls in a session.

/**
 * Inverse-frequency weighted word picker.
 * Tracks per-word usage counts and down-weights recently used words so output
 * has variety instead of repeating «блядь» on every sentence.
 *
 * Lifecycle: one instance per middleware closure; call reset() on session end.
 */
export class WordDiversifier {
  private readonly counts = new Map<string, number>();

  pick(arr: readonly string[]): string {
    if (arr.length === 0) throw new Error("WordDiversifier.pick: empty array");
    if (arr.length === 1) { this.increment(arr[0]); return arr[0]; }

    // Weight = 1 / (1 + count) — halves each time the word is used
    const weights = arr.map(w => 1 / (1 + (this.counts.get(w) ?? 0)));
    const total   = weights.reduce((a, b) => a + b, 0);

    let r = Math.random() * total;
    for (let i = 0; i < arr.length; i++) {
      r -= weights[i];
      if (r <= 0) { this.increment(arr[i]); return arr[i]; }
    }
    // Floating-point safety — return last element
    const last = arr[arr.length - 1];
    this.increment(last);
    return last;
  }

  /** Reset usage counts (call at session start/end). */
  reset(): void { this.counts.clear(); }

  /** Exposed for testing — get usage count for a word. */
  getCount(word: string): number { return this.counts.get(word) ?? 0; }

  private increment(word: string): void {
    this.counts.set(word, (this.counts.get(word) ?? 0) + 1);
  }
}


// ─── Russian Interjections (межметия) ────────────────────────────────────────
// Primary interjections: non-word vocal expressions of emotion.
// Lighter than mat — surprise, wonder, regret, relief.

const INTERJECT: Record<"prefix" | "suffix" | "mid", string[]> = {
  prefix: [
    "Ого!",        // ogo — wow
    "Ах!",         // akh — delight/surprise
    "Ух!",         // ukh — impressed
    "Ой!",         // oy — mild shock
    "Эй!",         // ey — hey!
    "Ба!",         // ba — well I never
    "Ишь ты!",     // ish ty — fancy that
    "Ну и ну!",    // nu i nu — well well
    "Вот те на!",  // vot te na — you don't say
    "О-го-го!",    // o-go-go — admiration/amazement
  ],
  suffix: [
    ", эх!",       // ekh — resignation/regret
    ", уф!",       // uf — relief/exhaustion
    ", ого!",      // ogo — impressed afterthought
    ", ай-яй-яй!", // ay-yay-yay — tsk tsk
    ", вот как!",  // vot kak — is that so!
    ", ишь!",      // ish — well I never (trailing)
  ],
  mid: [
    "ой",          // oy — brief flinch
    "ух",          // ukh — brief impression
    "эх",          // ekh — sigh
    "ну",          // nu — well...
    "ай",          // ay — brief pain
  ],
};

/**
 * Inject Russian interjections into already-translated text.
 *
 * @param text        Russian text to mutate.
 * @param prob        0–1 probability per opportunity.
 * @param diversifier WordDiversifier instance for this session.
 * @param bias        Optional emotion bias — overrides word pools with curated sets.
 */
function injectInterject(text: string, prob: number, diversifier: WordDiversifier, bias?: WordBias, biasWords?: BiasWordMap | null): string {
  if (!text || prob <= 0) return text ?? "";

  const bw = bias && bias !== "neutral" && biasWords ? biasWords[bias] : null;

  const sentences = text.split(/(?<=[.!?])\s+/).filter(Boolean);

  return sentences.map(sentence => {
    const scores = scorePlacement(sentence);

    if (Math.random() < prob * scores.prefix) {
      sentence = diversifier.pick(bw?.prefix ?? INTERJECT.prefix) + " " + sentence;
    }

    const clauses = sentence.split(",");
    const mutated = clauses.map((clause, i) => {
      if (i === 0) return clause;
      if (Math.random() < prob * scores.mid) return " " + diversifier.pick(bw?.standalone ?? INTERJECT.mid) + "," + clause;
      return clause;
    });
    sentence = mutated.join(",");

    if (Math.random() < prob * scores.suffix) {
      const stripped = sentence.replace(/[.!?]+$/, "");
      const punct    = sentence.match(/[.!?]+$/)?.[0] ?? ".";
      sentence = stripped + diversifier.pick(bw?.suffix ?? INTERJECT.suffix) + punct;
    }

    return sentence;
  }).join(" ");
}

// ─── Russian Mat ─────────────────────────────────────────────────────────────
// Curated expressions by insertion role.
// "interject" — standalone pause fillers that drop between phrases.
// "prefix"    — placed before a sentence/clause.
// "suffix"    — appended after a sentence/clause.

const MAT: Record<"interject" | "prefix" | "suffix", string[]> = {
  interject: [
    "блядь",         // blyad' — the universal filler
    "сука",          // suka — bitch (used as pure interjection)
    "ёб твою мать",  // classic
    "хуй",           // khuy — dick; used dismissively mid-sentence
    "пиздец",        // pizdets — it's fucked
    "ёпта",          // yopta — softer, like "damn it"
    "нихуя себе",    // ni khuya sebe — holy shit
    "блин",          // blin — milder, like "damn"
  ],
  prefix: [
    "ёбаный в рот,",      // yobany v rot — fucking hell,
    "какого хуя,",        // kakogo khuya — what the fuck,
    "мать твою,",         // mat' tvoyu — motherfucker,
    "ни хуя себе,",       // ni khuya sebe — holy shit,
    "чёрт возьми,",       // chyort voz'mi — damn it, (mild)
  ],
  suffix: [
    ", блядь",            // trailing blyad'
    ", сука",             // trailing suka
    ", пиздец",           // trailing pizdets
    ", ёб твою мать",     // trailing classic
    ", ёпта",             // trailing soft
    ", мать его",         // mat' ego — motherfucking (thing)
  ],
};

// ─── Expressive lengthening ───────────────────────────────────────────────────
// Vowels ranked by dramatic resonance: open/round first (А О У),
// sharp mid (Е Ё И), flat/soft last (Э Ю Я Ы).
const DRAMATIC_VOWELS = "АаОоУуЕеЁёИиЭэЮюЯяЫы";

/**
 * Stretch the most dramatically resonant vowel in a mat expression.
 * "блядь" → "бляааадь", "сука" → "суууука", "пиздец" → "пиздееец"
 *
 * Picks from the top half of vowels ranked by dramatic impact,
 * then repeats it `repeats` times in place.
 */
export function stretchExpression(expr: string, repeats: number): string {
  const positions: number[] = [];
  for (let i = 0; i < expr.length; i++) {
    if (DRAMATIC_VOWELS.includes(expr[i])) positions.push(i);
  }
  if (positions.length === 0) return expr;

  // Sort by dramatic priority — lower index in DRAMATIC_VOWELS = more resonant
  positions.sort((a, b) =>
    DRAMATIC_VOWELS.indexOf(expr[a]) - DRAMATIC_VOWELS.indexOf(expr[b])
  );

  // Pick randomly from the top half (most resonant vowels)
  const topN = Math.max(1, Math.ceil(positions.length / 2));
  const pos   = positions[Math.floor(Math.random() * topN)];
  const vowel = expr[pos];

  return expr.slice(0, pos) + vowel.repeat(repeats) + expr.slice(pos + 1);
}

/**
 * Inject mat into already-translated Russian text.
 *
 * Strategy:
 *  - Split on sentence boundaries and commas.
 *  - For each split point, roll a d100. Hit = insert an interject.
 *  - Also roll once per sentence for a prefix or suffix.
 *
 * @param text     Russian text to mutate.
 * @param prob     0–1 probability of injecting at each opportunity.
 */
/**
 * @param prob        0–1 injection probability per opportunity.
 * @param stretchProb 0–1 probability that an injected expression gets
 *                    expressive lengthening applied to its key vowel.
 * @param diversifier WordDiversifier instance for this session.
 * @param bias        Optional emotion bias — overrides word pools with curated sets.
 */
function injectMat(text: string, prob: number, stretchProb: number, diversifier: WordDiversifier, bias?: WordBias, biasWords?: BiasWordMap | null): string {
  if (!text || prob <= 0) return text ?? "";

  const bw = bias && bias !== "neutral" && biasWords ? biasWords[bias] : null;

  function pickMat(arr: readonly string[], biasArr?: readonly string[]): string {
    const pool = biasArr ?? arr;
    const expr = diversifier.pick(pool);
    if (stretchProb > 0 && Math.random() < stretchProb) {
      const repeats = STRETCH_REPEATS_MIN + Math.floor(Math.random() * (STRETCH_REPEATS_MAX - STRETCH_REPEATS_MIN + 1));
      return stretchExpression(expr, repeats);
    }
    return expr;
  }

  const sentences = text.split(/(?<=[.!?])\s+/).filter(Boolean);

  return sentences.map(sentence => {
    const scores = scorePlacement(sentence);

    if (Math.random() < prob * scores.prefix) {
      sentence = pickMat(MAT.prefix, bw?.prefix) + " " + sentence;
    }

    const clauses = sentence.split(",");
    const mutated = clauses.map((clause, i) => {
      if (i === 0) return clause;
      if (Math.random() < prob * scores.mid) return " " + pickMat(MAT.interject, bw?.standalone) + "," + clause;
      return clause;
    });
    sentence = mutated.join(",");

    if (Math.random() < prob * scores.suffix) {
      const stripped = sentence.replace(/[.!?]+$/, "");
      const punct    = sentence.match(/[.!?]+$/)?.[0] ?? ".";
      sentence = stripped + pickMat(MAT.suffix, bw?.suffix) + punct;
    }

    return sentence;
  }).join(" ");
}

// ─── Middleware pipeline ─────────────────────────────────────────────────────
//
// Koa-style async middleware for text transformation.
// Each step is a plain function (ctx, next) that mutates ctx.text.
// compose() runs the stack in order; each middleware calls next() to
// pass control to the next step.
//
// Usage:
//   const stack: TextMiddleware[] = [
//     makeTranslateMiddleware("en", "ru"),
//     makeMatMiddleware(0.35, 0.5),
//     makeInterjectMiddleware(0.25),
//   ];
//   const translator = new PipelineTranslator(stack, "ru");

export interface TextCtx {
  /** Original input text (never mutated). */
  readonly input: string;
  /** Current text — mutated by each middleware step. */
  text: string;
  /** Target language. */
  lang: "en" | "ru";
}

export type TextMiddleware = (
  ctx: TextCtx,
  next: () => Promise<void>,
) => Promise<void>;

/**
 * Compose a stack of TextMiddleware into a single async runner.
 * Executes left-to-right; each middleware must call next() to continue.
 */
export function compose(stack: TextMiddleware[]): (ctx: TextCtx) => Promise<void> {
  return (ctx) => {
    const dispatch = (i: number): Promise<void> =>
      i >= stack.length ? Promise.resolve() : stack[i](ctx, () => dispatch(i + 1));
    return dispatch(0);
  };
}

// ─── Middleware factories ─────────────────────────────────────────────────────

/**
 * Translate ctx.text → ctx.text using MyMemory.
 * Reads ctx.text (not ctx.input) so upstream middleware like
 * makeITGlossaryMiddleware can pre-process the text first.
 */
export function makeTranslateMiddleware(from: string, to: string): TextMiddleware {
  const t = from === to ? new IdentityTranslatorImpl() : new LibreTranslateTranslatorImpl(from, to);
  return async (ctx, next) => {
    ctx.text = await t.translate(ctx.text); // ctx.text may be pre-processed by glossary
    await next();
  };
}

// ─── IT Glossary middleware ─────────────────────────────────────────────────────────
//
// Pure string replacement, <1ms, zero network calls.
// Replaces unambiguous English IT terms with Russian developer loanwords
// BEFORE any translator sees the text, so downstream translation leaves
// the already-Russian terms untouched.
//
// Example:
//   'Deploy the service and commit the changes'
//   → 'деплоить the service and коммит the changes'
//   → MyMemory → 'деплоить сервис и коммит изменений'

type GlossaryEntry = [RegExp, string];

/**
 * English IT term → Russian developer loanword.
 * Only terms that are unambiguous in a coding context.
 * Verbs use the infinitive form; the translator handles conjugation.
 */
export const IT_GLOSSARY: GlossaryEntry[] = [
  // Version control
  [/\bpull\s+request(s)?\b/gi, 'пуллреквест'],
  [/\bcommit(s|ted|ting)?\b/gi, 'коммит'],
  [/\bmerge(d|ing)?\b/gi,       'мерж'],
  [/\brebase(d|ing)?\b/gi,      'ребейс'],
  [/\bcheckout\b/gi,            'чекаут'],
  [/\bstash\b/gi,               'стеш'],
  [/\bpush(ed|ing)?\b/gi,       'пуш'],
  [/\bpull(ed|ing)?\b/gi,       'пулл'],
  [/\bfork(ed|ing)?\b/gi,       'форк'],
  [/\bbranch(es)?\b/gi,         'ветка'],
  [/\b(git|Git)\b/g,            'гит'],

  // Deployment
  [/\bdeploy(ment|ed|ing|s)?\b/gi, 'деплой'],
  [/\brollback\b/gi,            'роллбек'],
  [/\bpipeline(s)?\b/gi,        'пайплайн'],
  [/\bCI\/CD\b/g,               'CI/CD'],
  [/\bstaging\b/gi,             'стейджинг'],
  [/\bprod(uction)?\b/gi,       'прод'],
  [/\bcontainer(s)?\b/gi,       'контейнер'],
  [/\b[Dd]ocker\b/g,            'докер'],
  [/\b[Kk]ubernetes\b/g,        'кубернетес'],
  [/\bk8s\b/g,                  'кубернетес'],

  // Code
  [/\bdebug(ging|ged)?\b/gi,    'дебаг'],
  [/\brefactor(ing|ed)?\b/gi,   'рефактор'],
  [/\bbuild(ing|s)?\b/gi,       'билд'],
  [/\btest(s|ing|ed)?\b/gi,     'тест'],
  [/\bfeature(s)?\b/gi,         'фича'],
  [/\bbug(s|fix)?\b/gi,         'баг'],
  [/\bfix(ed|ing|es)?\b/gi,     'фикс'],
  [/\bcache\b/gi,               'кэш'],
  [/\blog(s|ging)?\b/gi,        'лог'],

  // Architecture & infra
  [/\bbackend\b/gi,             'бэкенд'],
  [/\bfrontend\b/gi,            'фронтенд'],
  [/\bAPI\b/g,                  'API'],
  [/\bSQL\b/g,                  'SQL'],
  [/\bHTTP\b/g,                 'HTTP'],
  [/\bJSON\b/g,                 'JSON'],
  [/\bserver(s)?\b/gi,          'сервер'],
  [/\bdatabase\b/gi,            'база данных'],
  [/\b(PR|pr)\b/g,              'PR'],
  [/\brelease(s|d)?\b/gi,       'релиз'],
  [/\bcode\s+review\b/gi,       'ревью'],
  [/\breview(ed|ing|s)?\b/gi,   'ревью'],
  [/\bsprint(s)?\b/gi,          'спринт'],
  [/\bstandup\b/gi,             'стендап'],
  [/\bticket(s)?\b/gi,          'тикет'],
];

/**
 * IT glossary middleware — pre-process English IT terms to Russian loanwords.
 * Zero latency (<1ms), zero network, deterministic.
 * Must run BEFORE makeTranslateMiddleware.
 */
export function makeITGlossaryMiddleware(): TextMiddleware {
  return async (ctx, next) => {
    let text = ctx.text;
    for (const [pattern, replacement] of IT_GLOSSARY) {
      text = text.replace(pattern, replacement);
    }
    ctx.text = text;
    await next();
  };
}

/** Inject Russian mat into ctx.text after downstream runs. */
export function makeMatMiddleware(prob: number, stretch: number, bias?: WordBias, biasWords?: BiasWordMap | null): TextMiddleware {
  const diversifier = new WordDiversifier();
  return async (ctx, next) => {
    await next();
    ctx.text = injectMat(ctx.text, prob, stretch, diversifier, bias, biasWords);
  };
}

/** Inject Russian interjections into ctx.text after downstream runs. */
export function makeInterjectMiddleware(prob: number, bias?: WordBias, biasWords?: BiasWordMap | null): TextMiddleware {
  const diversifier = new WordDiversifier();
  return async (ctx, next) => {
    await next();
    ctx.text = injectInterject(ctx.text, prob, diversifier, bias, biasWords);
  };
}

/**
 * PipelineTranslator — implements Translator via a flat middleware stack.
 * Drop-in replacement for the nested decorator chain.
 *
 * @param stack  Ordered middleware steps.
 * @param lang   Target language (needed to set ctx.lang).
 */
export class PipelineTranslator implements Translator {
  private readonly run: (ctx: TextCtx) => Promise<void>;

  constructor(
    private readonly stack: TextMiddleware[],
    private readonly lang: "en" | "ru" = "en",
  ) {
    this.run = compose(stack);
  }

  async translate(input: string): Promise<string> {
    const ctx: TextCtx = { input, text: input, lang: this.lang };
    await this.run(ctx);
    return ctx.text;
  }
}

// ─── Private translator impls (used internally by middleware factories) ───────
// Named with *Impl suffix so the public export names stay clean.

class IdentityTranslatorImpl {
  async translate(text: string): Promise<string> { return text; }
}

/**
 * Self-hosted LibreTranslate (localhost:5000).
 * Run via: podman run -d -p 5000:5000 libretranslate/libretranslate --load-only en,ru
 *
 * Falls back to identity (passthrough) if server is not running.
 * Use inputLang === outputLang (passthrough mode) when no translation server
 * is available — have Claude write in the target language directly.
 */
class LibreTranslateTranslatorImpl {
  constructor(
    private readonly from: string,
    private readonly to: string,
    private readonly url: string = "http://localhost:5000",
  ) {}

  async translate(text: string): Promise<string> {
    try {
      const resp = await fetch(`${this.url}/translate`, {
        method:  "POST",
        headers: { "Content-Type": "application/json" },
        body:    JSON.stringify({ q: text, source: this.from, target: this.to, format: "text" }),
        signal:  AbortSignal.timeout(TRANSLATION_TIMEOUT_MS),
      });
      if (!resp.ok) return text;
      const data = await resp.json() as { translatedText: string };
      return data.translatedText || text;
    } catch {
      // Server not running — passthrough. Start with:
      // podman run -d -p 5000:5000 libretranslate/libretranslate --load-only en,ru
      return text;
    }
  }
}

// ─── Legacy decorator classes (kept for backwards compat & existing tests) ────

export class IdentityTranslator implements Translator {
  async translate(text: string): Promise<string> {
    return text;
  }
}

/** @deprecated Use LibreTranslateTranslator (self-hosted) or set inputLang===outputLang for passthrough. */
export class MyMemoryTranslator implements Translator {
  constructor(private readonly from: string, private readonly to: string) {}
  async translate(text: string): Promise<string> {
    console.warn("[foni] MyMemoryTranslator is deprecated — it uses a cloud API. Use LibreTranslateTranslator or passthrough mode.");
    return text;
  }
}

/**
 * Self-hosted LibreTranslate translator (implements Translator interface).
 * Requires: podman run -d -p 5000:5000 libretranslate/libretranslate --load-only en,ru
 */
export class LibreTranslateTranslator implements Translator {
  private readonly impl: LibreTranslateTranslatorImpl;
  constructor(
    private readonly from: string,
    private readonly to: string,
    url = "http://localhost:5000",
  ) {
    this.impl = new LibreTranslateTranslatorImpl(from, to, url);
  }
  async translate(text: string): Promise<string> {
    return this.impl.translate(text);
  }
}

/**
 * MatTranslator — wraps any Translator and randomly injects Russian mat
 * at natural pause points after the inner translation completes.
 *
 * Only meaningful when the inner translator produces Russian output.
 *
 * @param inner    The real translator to delegate to first.
 * @param prob     0–1 injection probability per opportunity (default 0.35).
 */
/**
 * MatTranslator — wraps any Translator and randomly injects Russian mat
 * at natural pause points after the inner translation completes.
 *
 * @param inner            Translator to delegate to first.
 * @param probability      0–1 injection probability per opportunity (default 0.35).
 * @param stretchProbability 0–1 chance each injected word gets expressive
 *                         lengthening on its most resonant vowel (default 0.5).
 */
export class MatTranslator implements Translator {
  private readonly diversifier = new WordDiversifier();
  constructor(
    private readonly inner: Translator,
    public probability: number = 0.35,
    public stretchProbability: number = 0.5,
  ) {}

  async translate(text: string): Promise<string> {
    const result = await this.inner.translate(text);
    return injectMat(result, this.probability, this.stretchProbability, this.diversifier);
  }
}

/**
 * InterjectTranslator — wraps any Translator and randomly injects Russian
 * primary interjections (межметия) at natural pause points.
 *
 * Lighter than mat — surprise, wonder, regret, relief.
 * Composes naturally on top of MatTranslator:
 *   MyMemoryTranslator → MatTranslator → InterjectTranslator
 *
 * @param inner       Translator to delegate to first.
 * @param probability 0–1 injection probability per opportunity (default 0.25).
 */
export class InterjectTranslator implements Translator {
  private readonly diversifier = new WordDiversifier();
  constructor(
    private readonly inner: Translator,
    public probability: number = 0.25,
  ) {}

  async translate(text: string): Promise<string> {
    const result = await this.inner.translate(text);
    return injectInterject(result, this.probability, this.diversifier);
  }
}
