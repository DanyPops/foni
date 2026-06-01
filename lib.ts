// lib.ts — re-exports from core/ for backwards compatibility.
// Tests and other files still importing from here continue to work.
// @deprecated Import directly from core/stream.ts or core/config.ts.

export { stripMarkdown, drainChunks, freshState, resolveBacktickRun } from "./core/stream.ts";
export type { DrainResult, StreamState } from "./core/stream.ts";


