/**
 * Shared domain types used by both core/ and pipeline/.
 * Lives in core/ because they are driven by domain logic (emotion state),
 * not by pipeline mechanics. Moving them here breaks the core↔pipeline cycle.
 */

/** Word selection bias — driven by the detected emotion state. */
export type WordBias =
  | "aggressive"    // heat-3 mat only
  | "commiseration" // empathetic mat + prison jargon
  | "mockery"       // ironic/dismissive interjections
  | "excitement"    // ого!, нихуя себе!, нехило!
  | "neutral";

/** Word pool for a single bias category (suffix / standalone / prefix positions). */
export interface BiasWordSet {
  suffix:     string[];
  standalone: string[];
  prefix:     string[];
}

/**
 * Full bias word map — injected into middleware factories from core/emotion.ts.
 * Lives here so pipeline/translators.ts never imports from core/.
 */
export type BiasWordMap = Record<WordBias, BiasWordSet>;
