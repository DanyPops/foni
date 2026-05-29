// ─── Markdown stripping ───────────────────────────────────────────────────────

export function stripMarkdown(text: string): string {
  return (
    text
      .replace(/\n?```[\s\S]*?```\n?/g, "\n")
      .replace(/`[^`]+`/g, "")
      .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
      .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
      .replace(/^#{1,6}\s+/gm, "")
      .replace(/(\*{1,3}|_{1,3})(.+?)\1/g, "$2")
      .replace(/^>\s*/gm, "")
      .replace(/^[-*_]{3,}\s*$/gm, "")
      .replace(/^[\s]*[-*+]\s+/gm, "")
      .replace(/^[\s]*\d+\.\s+/gm, "")
      .replace(/\n{3,}/g, "\n\n")
      .trim()
  );
}

// ─── Sentence / paragraph chunking ───────────────────────────────────────────

export interface DrainResult {
  chunks: string[];
  remainder: string;
}

export function drainChunks(text: string): DrainResult {
  const chunks: string[] = [];
  let remaining = text;

  const paraRe = /\n\n+/;
  // Language-agnostic split: any .!? followed by whitespace.
  // The previous lookahead (?=[A-Z]) only matched English capitals,
  // silently treating entire Russian paragraphs as single unsplit chunks.
  const sentRe = /([.!?!?])\s+/;

  let safety = 0;
  while (safety++ < 50) {
    const paraIdx = paraRe.exec(remaining)?.index ?? Infinity;
    const sentMatch = sentRe.exec(remaining);
    const sentIdx = sentMatch ? sentMatch.index + 1 : Infinity;

    if (paraIdx === Infinity && sentIdx === Infinity) break;

    if (paraIdx <= sentIdx) {
      const chunk = remaining.slice(0, paraIdx).trim();
      if (chunk.length > 2) chunks.push(chunk);
      remaining = remaining.slice(paraIdx).replace(/^\n+/, "");
    } else if (sentMatch) {
      const cut = sentMatch.index + 1;
      const chunk = remaining.slice(0, cut).trim();
      if (chunk.length > 2) chunks.push(chunk);
      remaining = remaining.slice(cut).replace(/^\s+/, "");
    } else {
      break;
    }
  }

  return { chunks, remainder: remaining };
}

// ─── Stream state ─────────────────────────────────────────────────────────────

export interface StreamState {
  buffer: string;
  codeDepth: number;
  inInlineCode: boolean;
  backtickRun: number;
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

// ─── FakeYou helpers ──────────────────────────────────────────────────────────

export function buildFakeYouHeaders(apiKey: string): Record<string, string> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    Accept: "application/json",
  };
  if (apiKey) headers["Authorization"] = `Bearer ${apiKey}`;
  return headers;
}

export const FAKEYOU_CDN = "https://storage.googleapis.com/vocodes-public";
