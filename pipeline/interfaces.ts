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
