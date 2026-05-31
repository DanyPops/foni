/**
 * core/interfaces.ts — Abstract ports for the pipeline layer.
 *
 * FoniEngine depends only on these interfaces, never on concrete pipeline classes.
 * This breaks the core ↔ pipeline import cycle that Locus detects.
 *
 * Concrete implementations:
 *   FacadePort ← pipeline/speak-facade.ts SpeakFacade
 */

import type { AudioProcessor, Translator } from "../pipeline/interfaces.ts";

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
