/**
 * core/interfaces.ts — Domain port interfaces.
 *
 * These are pure Strategy abstractions owned by the domain layer.
 * Concrete implementations live in pipeline/ and backends/.
 * No imports from pipeline/ — this is the DIP boundary.
 */

import type { FoniConfig }  from "./config.ts";
import type { EmotionState } from "./emotion.ts";

// ─── Core Strategy ports ──────────────────────────────────────────────────────

/** Translates text (EN→RU, mat injection, glossary middleware). */
export interface Translator {
  translate(text: string): Promise<string>;
}

/** Transforms a raw audio buffer (RVC, DSP chain, identity). */
export interface AudioProcessor {
  process(input: Buffer): Promise<Buffer>;
}

/** Plays a PCM WAV buffer to the system audio device. */
export interface Player {
  play(buf: Buffer): Promise<void>;
}

// ─── Factory types (injected into FoniEngine) ─────────────────────────────────

/**
 * Builds the translator middleware stack for the current config + emotion state.
 * Called by FoniEngine.rebuildTranslator() when config or emotion changes.
 */
export type TranslatorFactory = (config: FoniConfig, emotion: EmotionState) => Translator;

/**
 * Constructs the full audio pipeline facade.
 * Called by FoniEngine.buildFacade(); encapsulates backend + processor + player.
 */
export type FacadeFactory = (config: FoniConfig, translator: Translator) => Promise<FacadePort | null>;

/**
 * Reconstructs the audio processor chain (RVC, smoothing, breath).
 * Called when the RVC model or breath settings change.
 */
export type ProcessorFactory = (config: FoniConfig) => AudioProcessor;

// ─── FacadePort ───────────────────────────────────────────────────────────────

/** Minimal surface of SpeakFacade that FoniEngine actually calls. */
export interface FacadePort {
  readonly backendName: string;
  speak(text: string, log?: (msg: string) => void): Promise<void>;
  stop(): void;
  swapTranslator(t: Translator): void;
  swapProcessor(p: AudioProcessor): void;
  setOpts(opts: { voice?: string; speed?: number }): void;
  cacheStats(): string;
  cache: { clear(): void };
}
