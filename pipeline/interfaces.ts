/**
 * pipeline/interfaces.ts — TTS pipeline strategy interfaces.
 *
 * Translator, AudioProcessor, Player are domain ports owned by core/interfaces.ts.
 * Re-exported here so all existing pipeline/ consumers keep working unchanged.
 * TTSBackend and SynthOptions are TTS-specific and remain here.
 */

// Domain ports — re-exported from core/ for backward compatibility.
export type { Translator, AudioProcessor, Player } from "../core/interfaces.ts";

// Domain word-bias types — re-exported from core/types.ts.
export type { WordBias, BiasWordSet, BiasWordMap } from "../core/types.ts";

// ─── TTS-specific ─────────────────────────────────────────────────────────────

export interface SynthOptions {
  voice: string;
  speed: number;
}

export interface TTSBackend {
  readonly name: string;
  isAvailable(): Promise<boolean>;
  /** Synthesize text and return raw audio as a Buffer. */
  synthesize(text: string, opts: SynthOptions): Promise<Buffer>;
}
