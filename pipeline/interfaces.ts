/**
 * Core abstractions for the Foni TTS pipeline.
 *
 * Pipeline:  text → Translator → TTSBackend → AudioProcessor → Player
 *
 * Each stage is a Strategy — swap implementations without touching the Facade.
 */

// ─── Strategy: Translator ─────────────────────────────────────────────────────

export interface Translator {
  translate(text: string): Promise<string>;
}

// ─── Strategy: TTSBackend ────────────────────────────────────────────────────

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

// ─── Strategy: AudioProcessor ────────────────────────────────────────────────

export interface AudioProcessor {
  /** Transform audio buffer. Must never throw — returns input on failure. */
  process(input: Buffer): Promise<Buffer>;
}

// ─── Player ──────────────────────────────────────────────────────────────────

export interface Player {
  play(buf: Buffer): Promise<void>;
}

// ─── Shared domain types ──────────────────────────────────────────────────────
// Placed here (not core/) so pipeline/ can use them without importing from core/.

/**
 * Word selection bias — driven by the detected emotion state.
 * The values mirror EmotionState.bias but live in pipeline/interfaces.ts
 * so translators.ts never needs to import from core/.
 */
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
