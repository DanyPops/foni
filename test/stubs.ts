/**
 * test/stubs.ts — shared stub implementations for all test files.
 *
 * Single source of truth. Import instead of redefining per file.
 */

import type { AudioProcessor, Player, SynthOptions, TTSBackend } from "../pipeline/interfaces.ts";
import type { FacadeFactory, ProcessorFactory, TranslatorFactory } from "../core/engine.ts";
import { EspeakBackend }        from "../backends/espeak.ts";
import { SpeakFacade }          from "../pipeline/speak-facade.ts";
import { PipelineTranslator, makeITGlossaryMiddleware } from "../pipeline/translators.ts";
import { SmoothingProcessor, RVCProcessor, IdentityProcessor } from "../pipeline/processors.ts";
import { Env } from "./env.ts";

// ─── Null / identity stubs ────────────────────────────────────────────────────

/** Minimal 44-byte WAV (header only, silence). */
const EMPTY_WAV = Buffer.alloc(44);

export const stubBackend: TTSBackend = {
  name: "stub",
  isAvailable: async () => true,
  synthesize: async (_t: string, _o: SynthOptions): Promise<Buffer> => EMPTY_WAV,
};

export const nullBackend: TTSBackend = {
  name: "null",
  isAvailable: async () => false,
  synthesize: async () => EMPTY_WAV,
};

export class NullProcessor implements AudioProcessor {
  async process(b: Buffer): Promise<Buffer> { return b; }
}

export class NullPlayer implements Player {
  readonly played: number[] = [];
  async play(b: Buffer): Promise<void> { this.played.push(b.length); }
}

// ─── Timing player ────────────────────────────────────────────────────────────

export interface PlayEvent { t: number; bytes: number; }

export class TimingPlayer implements Player {
  readonly events: PlayEvent[] = [];
  get firstPlayMs()  { return this.events[0]?.t       ?? null; }
  get lastPlayMs()   { return this.events.at(-1)?.t   ?? null; }
  get totalChunks()  { return this.events.length; }
  get totalBytes()   { return this.events.reduce((s, e) => s + e.bytes, 0); }
  async play(buf: Buffer) { this.events.push({ t: Date.now(), bytes: buf.length }); }

  /** Wait until 1 s of silence after last chunk, or `timeoutMs`. */
  async drain(timeoutMs = 90_000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      await new Promise(r => setTimeout(r, 250));
      if (this.lastPlayMs !== null && Date.now() - this.lastPlayMs > 1_000) return;
    }
  }
}

// ─── Factory presets ──────────────────────────────────────────────────────────

export const nullProcessorFactory: ProcessorFactory = () => new NullProcessor();

export const glossaryTranslatorFactory: TranslatorFactory = (cfg, _emotion) =>
  new PipelineTranslator([makeITGlossaryMiddleware()], cfg.outputLang);

/**
 * espeak + NullProcessor + given player. Returns null if espeak is unavailable.
 * Used by engine-level tests that need real synthesis without RVC overhead.
 */
export function makeEspeakFactory(player: Player): FacadeFactory {
  return async (_cfg, translator, _emotion) => {
    const backend = new EspeakBackend(Env.VOICE);
    if (!await backend.isAvailable()) return null;
    return new SpeakFacade(translator, backend, new NullProcessor(), player, {
      voice: Env.VOICE, speed: 1.15,
    });
  };
}

/**
 * espeak + full Smoothing+RVC processor + given player.
 * Requires foni-synth reachable at `synthUrl`.
 */
export function makeRvcFactory(player: Player, synthUrl: string): FacadeFactory {
  return async (_cfg, translator, _emotion) => {
    const backend   = new EspeakBackend(Env.VOICE);
    if (!await backend.isAvailable()) return null;
    const processor = new SmoothingProcessor(new RVCProcessor(synthUrl), {}, synthUrl);
    return new SpeakFacade(translator, backend, processor, player, {
      voice: Env.VOICE, speed: 1.15,
    });
  };
}
