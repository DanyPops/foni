import type { Translator } from "./interfaces.ts";

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
function injectMat(text: string, prob: number): string {
  if (prob <= 0) return text;

  // Split into sentences
  const sentences = text.split(/(?<=[.!?])\s+/).filter(Boolean);

  return sentences.map(sentence => {
    // Prefix roll
    if (Math.random() < prob * 0.5) {
      sentence = pick(MAT.prefix) + " " + sentence;
    }

    // Inject at commas: split on comma, roll per gap
    const clauses = sentence.split(",");
    const mutated = clauses.map((clause, i) => {
      if (i === 0) return clause;
      if (Math.random() < prob) {
        return " " + pick(MAT.interject) + "," + clause;
      }
      return clause;
    });
    sentence = mutated.join(",");

    // Suffix roll — only if sentence doesn't already end in punctuation mat
    if (Math.random() < prob * 0.4) {
      const stripped = sentence.replace(/[.!?]+$/, "");
      const punct    = sentence.match(/[.!?]+$/)?.[0] ?? ".";
      sentence = stripped + pick(MAT.suffix) + punct;
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
export class MatTranslator implements Translator {
  constructor(
    private readonly inner: Translator,
    public probability: number = 0.35,
  ) {}

  async translate(text: string): Promise<string> {
    const result = await this.inner.translate(text);
    return injectMat(result, this.probability);
  }
}
