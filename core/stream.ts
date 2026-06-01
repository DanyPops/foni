// ─── Stream state ─────────────────────────────────────────────────────────────
//
// Tracks backtick/code-block depth while consuming a streaming LLM delta.
// Zero pi dependencies — pure domain logic.

export interface StreamState {
  buffer:       string;
  codeDepth:    number;
  inInlineCode: boolean;
  backtickRun:  number;
}

export function freshState(): StreamState {
  return { buffer: "", codeDepth: 0, inInlineCode: false, backtickRun: 0 };
}

export function resolveBacktickRun(state: StreamState): void {
  const run = state.backtickRun;
  state.backtickRun = 0;
  if (run === 0) return;
  if (run >= 3) {
    state.codeDepth = state.codeDepth > 0 ? 0 : 1;
  } else if (state.codeDepth === 0) {
    state.inInlineCode = !state.inInlineCode;
  }
}

// ─── Text chunking ────────────────────────────────────────────────────────────

export interface DrainResult {
  chunks:    string[];
  remainder: string;
}

/**
 * Split buffered text into speakable chunks at paragraph and sentence
 * boundaries. Language-agnostic: handles both English and Cyrillic.
 */
export function drainChunks(text: string): DrainResult {
  const chunks: string[] = [];
  let remaining = text;

  const paraRe = /\n\n+/;
  const sentRe = /([.!?!?])\s+/;

  let safety = 0;
  while (safety++ < 50) {
    const paraIdx  = paraRe.exec(remaining)?.index ?? Infinity;
    const sentMatch = sentRe.exec(remaining);
    const sentIdx  = sentMatch ? sentMatch.index + 1 : Infinity;

    if (paraIdx === Infinity && sentIdx === Infinity) break;

    if (paraIdx <= sentIdx) {
      const chunk = remaining.slice(0, paraIdx).trim();
      if (chunk.length > 2) chunks.push(chunk);
      remaining = remaining.slice(paraIdx).replace(/^\n+/, "");
    } else if (sentMatch) {
      const cut   = sentMatch.index + 1;
      const chunk = remaining.slice(0, cut).trim();
      if (chunk.length > 2) chunks.push(chunk);
      remaining = remaining.slice(cut).replace(/^\s+/, "");
    } else {
      break;
    }
  }

  return { chunks, remainder: remaining };
}

// ─── Markdown stripping ───────────────────────────────────────────────────────

export function stripMarkdown(text: string): string {
  return text
    // Complete fenced code blocks
    .replace(/\n?```[\s\S]*?```\n?/g, "\n")
    // Unclosed fenced code block (mid-stream: ``` opened but not closed yet)
    .replace(/```[\s\S]*/g, "")
    // Inline code
    .replace(/`[^`]+`/g, "")
    // Images, links
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    // Headers, emphasis, blockquote, rules, lists
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/(\*{1,3}|_{1,3})(.+?)\1/g, "$2")
    .replace(/^>\s*/gm, "")
    .replace(/^[-*_]{3,}\s*$/gm, "")
    .replace(/^[\s]*[-*+]\s+/gm, "")
    .replace(/^[\s]*\d+\.\s+/gm, "")
    // Shell/regex special sequences that produce audible noise:
    //   \| (grep alternation), \n \t (escape sequences), trailing backslash
    .replace(/\\[|ntrfv\\]/g, " ")
    .replace(/\\\s*$/gm, "")
    // /path-like-tokens that aren't prose (e.g. /metrics, /tts, /no_think)
    // Only strip when surrounded by non-letter context (not mid-word fractions)
    .replace(/(?<![\w])\/[a-z][a-z0-9_\-]*/gi, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
