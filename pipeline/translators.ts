import type { Translator } from "./interfaces.ts";

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
 * @param text     Russian text to mutate.
 * @param prob     0–1 probability per opportunity.
 */
function injectInterject(text: string, prob: number): string {
  if (prob <= 0) return text;

  const sentences = text.split(/(?<=[.!?])\s+/).filter(Boolean);

  return sentences.map(sentence => {
    // Prefix: prepend an exclamation before the sentence
    if (Math.random() < prob * 0.45) {
      sentence = pick(INTERJECT.prefix) + " " + sentence;
    }

    // Mid: drop a soft filler at comma breaks
    const clauses = sentence.split(",");
    const mutated = clauses.map((clause, i) => {
      if (i === 0) return clause;
      if (Math.random() < prob * 0.5) return " " + pick(INTERJECT.mid) + "," + clause;
      return clause;
    });
    sentence = mutated.join(",");

    // Suffix: trail off with an exclamation
    if (Math.random() < prob * 0.35) {
      const stripped = sentence.replace(/[.!?]+$/, "");
      const punct    = sentence.match(/[.!?]+$/)?.[0] ?? ".";
      sentence = stripped + pick(INTERJECT.suffix) + punct;
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

function pick<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)];
}

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
 */
function injectMat(text: string, prob: number, stretchProb: number): string {
  if (prob <= 0) return text;

  function pickMat(arr: string[]): string {
    const expr = pick(arr);
    if (stretchProb > 0 && Math.random() < stretchProb) {
      const repeats = 2 + Math.floor(Math.random() * 3); // 2, 3, or 4
      return stretchExpression(expr, repeats);
    }
    return expr;
  }

  const sentences = text.split(/(?<=[.!?])\s+/).filter(Boolean);

  return sentences.map(sentence => {
    if (Math.random() < prob * 0.5) {
      sentence = pickMat(MAT.prefix) + " " + sentence;
    }

    const clauses = sentence.split(",");
    const mutated = clauses.map((clause, i) => {
      if (i === 0) return clause;
      if (Math.random() < prob) return " " + pickMat(MAT.interject) + "," + clause;
      return clause;
    });
    sentence = mutated.join(",");

    if (Math.random() < prob * 0.4) {
      const stripped = sentence.replace(/[.!?]+$/, "");
      const punct    = sentence.match(/[.!?]+$/)?.[0] ?? ".";
      sentence = stripped + pickMat(MAT.suffix) + punct;
    }

    return sentence;
  }).join(" ");
}

export class IdentityTranslator implements Translator {
  async translate(text: string): Promise<string> {
    return text;
  }
}

export class MyMemoryTranslator implements Translator {
  constructor(private readonly from: string, private readonly to: string) {}

  async translate(text: string): Promise<string> {
    try {
      const url = `https://api.mymemory.translated.net/get?q=${encodeURIComponent(text)}&langpair=${this.from}|${this.to}`;
      const resp = await fetch(url, { signal: AbortSignal.timeout(5_000) });
      if (!resp.ok) return text;
      const data = await resp.json() as { responseData: { translatedText: string } };
      return data.responseData.translatedText || text;
    } catch {
      return text;
    }
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
  constructor(
    private readonly inner: Translator,
    public probability: number = 0.35,
    public stretchProbability: number = 0.5,
  ) {}

  async translate(text: string): Promise<string> {
    const result = await this.inner.translate(text);
    return injectMat(result, this.probability, this.stretchProbability);
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
  constructor(
    private readonly inner: Translator,
    public probability: number = 0.25,
  ) {}

  async translate(text: string): Promise<string> {
    const result = await this.inner.translate(text);
    return injectInterject(result, this.probability);
  }
}
