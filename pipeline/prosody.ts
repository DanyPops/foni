/**
 * pipeline/prosody.ts — Rule-based Russian text → SSML prosody annotator.
 *
 * Wraps plain text in SSML with:
 *   - <break> tags at punctuation boundaries (comma, dash, period, ellipsis…)
 *   - <prosody rate pitch range> variation per sentence
 *   - Question/exclamation detection for intonation shape
 *   - Phrase-final slowing (last clause before sentence end)
 *
 * Output is compatible with espeak-ng -m (markup mode).
 *
 * Research basis:
 *   - Google study: raters not sensitive to ±20% pause variation — rules don't
 *     need to be perfect, just in the right ballpark.
 *   - KTH breathing paper: natural breath positions = after long phrases (>5 words)
 *     before clause-initial positions.
 *   - SSML arXiv 2508.17494: simple break+prosody rules move MOS 3.20→3.87.
 */

// ─── Break durations (ms) ────────────────────────────────────────────────────

/**
 * Silence inserted for each punctuation type.
 * Default values mirror Rust ssml.rs constants (authoritative source).
 * Refreshed at runtime from GET /ssml-params when foni-synth is available.
 */
const BREAKS_DEFAULT = {
  comma:       150,
  semicolon:   220,
  dash:        200,
  colon:       180,
  ellipsis:    420,
  question:    350,
  exclamation: 300,
  period:      320,
} as const;

type BreakMap = { -readonly [K in keyof typeof BREAKS_DEFAULT]: number };

let BREAKS: BreakMap = { ...BREAKS_DEFAULT };

/**
 * Fetch break durations from the Rust server and update the in-memory map.
 * Fire-and-forget — falls back to defaults if the server is unreachable.
 */
export async function syncBreaksFrom(url: string): Promise<void> {
  try {
    const r = await fetch(`${url}/ssml-params`, { signal: AbortSignal.timeout(2_000) });
    if (!r.ok) return;
    const data = await r.json() as Record<string, number>;
    for (const key of Object.keys(BREAKS) as (keyof BreakMap)[]) {
      if (typeof data[key] === "number") BREAKS[key] = data[key];
    }
  } catch { /* keep defaults */ }
}

/** Russian coordinating conjunctions that often precede independent clauses. */
const CLAUSE_CONJUNCTIONS = /(?<=[,;—]\s*)(?:и|а|но|да(?!\s+не)|или|либо|зато|однако|притом|причём)\b/gu;

// ─── Prosody parameter ranges ─────────────────────────────────────────────────

/** Rate variation ±X% from baseline per sentence. Seeded from sentence index. */
const RATE_JITTER_PCT = 6;


/** Rate reduction for phrase-final clause (last clause before sentence end). */
const PHRASE_FINAL_RATE_REDUCTION_PCT = 8;

// ─── Sentence splitter ────────────────────────────────────────────────────────

interface Sentence {
  text:        string;
  terminator:  "." | "!" | "?" | "…" | "";
}

/** Split text into sentences, preserving their terminators. */
function splitSentences(text: string): Sentence[] {
  const sentences: Sentence[] = [];
  // Split on sentence-ending punctuation, keeping the delimiter
  const parts = text.split(/(?<=[.!?…])\s+/u);
  for (const part of parts) {
    const trimmed = part.trim();
    if (!trimmed) continue;
    const last = trimmed.slice(-1) as "." | "!" | "?" | "…";
    const isTerminator = [".", "!", "?", "…"].includes(last);
    sentences.push({
      text:       trimmed,
      terminator: isTerminator ? last : "",
    });
  }
  if (sentences.length === 0) {
    sentences.push({ text: text.trim(), terminator: "" });
  }
  return sentences;
}

// ─── Deterministic jitter (no Math.random — reproducible per sentence) ────────

/** Simple hash of a string → 0..1. Deterministic so output is stable. */
function hashStr(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return (h >>> 0) / 0xFFFFFFFF;
}

// ─── SSML builder ─────────────────────────────────────────────────────────────

function escapeXml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function breakTag(ms: number): string {
  return `<break time="${ms}ms"/>`;
}

/**
 * Annotate punctuation inside a sentence with SSML break tags.
 * Handles: , ; — … : mid-sentence
 */
function annotatePunctuation(text: string): string {
  return text
    // Em dash — (en-dash – also)
    .replace(/\s*[—–]\s*/g, ` ${breakTag(BREAKS.dash)} `)
    // Ellipsis … (and three dots)
    .replace(/\.{3}|…/g, `…${breakTag(BREAKS.ellipsis)}`)
    // Semicolon
    .replace(/;\s*/g, `; ${breakTag(BREAKS.semicolon)}`)
    // Colon (not inside time patterns like 12:30)
    .replace(/(?<!\d):\s*(?!\d)/g, `: ${breakTag(BREAKS.colon)}`)
    // Comma
    .replace(/,\s*/g, `, ${breakTag(BREAKS.comma)}`);
}

/**
 * Apply phrase-final rate slowing to the last clause of a sentence.
 * Splits on the last comma/dash before the end and wraps the tail.
 */
function applyPhraseFinalSlowing(text: string): string {
  // Find the last clause boundary (the last break tag insertion point)
  const lastBreakIdx = text.lastIndexOf("<break");
  if (lastBreakIdx === -1) return text;

  // Everything after the last break is the phrase-final clause
  const after = text.slice(lastBreakIdx);
  const closeTag = after.indexOf("/>") + 2;
  if (closeTag < 2) return text;

  const finalClause = after.slice(closeTag).trim();
  if (!finalClause) return text;

  const rate = 100 - PHRASE_FINAL_RATE_REDUCTION_PCT;
  const wrapped = `<prosody rate="${rate}%">${finalClause}</prosody>`;
  return text.slice(0, lastBreakIdx + closeTag) + " " + wrapped;
}

/**
 * Build the <prosody> wrapper for a complete sentence.
 * Varies rate and range deterministically from the sentence content.
 * pitch= is intentionally omitted: bare integer pitch= values in espeak SSML
 * shift the fundamental frequency into the wrong range (~3× natural F0).
 * range= is sufficient for expressiveness; absolute pitch is set by espeak voice choice.
 */
function sentenceProsody(sentence: Sentence, idx: number): { rate: number; range: string } {
  const rng = hashStr(sentence.text + idx);
  const rate = Math.round(100 + (rng - 0.5) * 2 * RATE_JITTER_PCT);
  if (sentence.terminator === "?") return { rate, range: "high" };
  if (sentence.terminator === "!") return { rate: rate + 3, range: "x-high" };
  return { rate, range: "medium" };
}

// ─── Public API ───────────────────────────────────────────────────────────────

export interface ProsodyOptions {
  /** Enable SSML break tags at punctuation. Default: true. */
  breaks?: boolean;
  /** Enable per-sentence prosody rate/pitch variation. Default: true. */
  prosodyVariation?: boolean;
  /** Enable phrase-final slowing. Default: true. */
  phraseFinalSlowing?: boolean;
}

/**
 * Annotate Russian plain text with SSML prosody markup.
 * Returns a full <speak>...</speak> SSML document for espeak-ng -m.
 */
export function annotateProsody(text: string, opts: ProsodyOptions = {}): string {
  if (!text) return "<speak></speak>";
  const {
    breaks            = true,
    prosodyVariation  = true,
    phraseFinalSlowing = true,
  } = opts;

  const sentences = splitSentences(text);
  const parts: string[] = [];

  for (let i = 0; i < sentences.length; i++) {
    const s = sentences[i]!;
    let body = escapeXml(s.text);

    // 1. Annotate in-sentence punctuation with break tags
    if (breaks) {
      body = annotatePunctuation(body);
    }

    // 2. Phrase-final slowing (last clause before terminator)
    // Requires prosodyVariation — uses <prosody> tags internally
    if (phraseFinalSlowing && breaks && prosodyVariation) {
      body = applyPhraseFinalSlowing(body);
    }

    // 3. Per-sentence prosody wrapper
    if (prosodyVariation) {
      const { rate, range } = sentenceProsody(s, i);
      body = `<prosody rate="${rate}%" range="${range}">${body}</prosody>`;
    }

    // 4. Sentence-final break (except last sentence)
    const termMs = s.terminator === "?"  ? BREAKS.question    :
                   s.terminator === "!"  ? BREAKS.exclamation :
                   s.terminator === "…"  ? BREAKS.ellipsis    :
                   s.terminator === "."  ? BREAKS.period      : 0;

    parts.push(body);
    if (termMs > 0 && i < sentences.length - 1) {
      parts.push(breakTag(termMs));
    }
  }

  return `<speak>${parts.join("\n")}</speak>`;
}

/**
 * Detect whether a string is already SSML (starts with <speak>).
 */
export function isSsml(text: string): boolean {
  return text.trimStart().startsWith("<speak");
}


