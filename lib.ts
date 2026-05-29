// lib.ts — re-exports from core/ for backwards compatibility.
// Tests and other files still importing from here continue to work.
// @deprecated Import directly from core/stream.ts or core/config.ts.

export { stripMarkdown, drainChunks, freshState, resolveBacktickRun } from "./core/stream.ts";
export type { DrainResult, StreamState } from "./core/stream.ts";

// FakeYou helpers — moved to backends/fakeyou.ts but re-exported for test compat
export { FAKEYOU_CDN } from "./backends/fakeyou.ts";

export function buildFakeYouHeaders(apiKey: string): Record<string, string> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    Accept: "application/json",
  };
  if (apiKey) headers["Authorization"] = `Bearer ${apiKey}`;
  return headers;
}
