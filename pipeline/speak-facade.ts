/**
 * SpeakFacade — unified TTS pipeline.
 *
 * Composes: Translator → TTSBackend → AudioProcessor → Player
 *
 * Callers interact only with this facade. They never touch individual
 * backends, processors, or the player directly.
 */

import { stripMarkdown } from "../lib.ts";
import type { AudioProcessor, Player, SynthOptions, Translator, TTSBackend } from "./interfaces.ts";

export type Log = (msg: string) => void;

export class SpeakFacade {
  constructor(
    private translator: Translator,
    private backend: TTSBackend,
    private processor: AudioProcessor,
    private player: Player,
    private opts: SynthOptions,
  ) {}

  get backendName(): string { return this.backend.name; }

  swapTranslator(t: Translator): void { this.translator = t; }
  swapBackend(b: TTSBackend): void { this.backend = b; }
  swapProcessor(p: AudioProcessor): void { this.processor = p; }
  setOpts(opts: Partial<SynthOptions>): void { this.opts = { ...this.opts, ...opts }; }

  async speak(rawText: string, log?: Log): Promise<void> {
    const emit = log ?? ((_m: string) => {});

    const clean = stripMarkdown(rawText).trim();
    if (clean.length < 3) { emit("skipped: text too short after stripping"); return; }

    const text = await this.translator.translate(clean);
    emit(`backend=${this.backend.name} text="${text.slice(0, 50)}…"`);

    try {
      const audio = await this.backend.synthesize(text, this.opts);
      emit(`synthesized ${audio.length} bytes`);

      const processed = await this.processor.process(audio);
      if (processed !== audio) emit(`processed via ${this.processor.constructor.name}`);

      await this.player.play(processed);
      emit(`played ${processed.length} bytes`);
    } catch (e: any) {
      emit(`ERROR: ${e?.message}`);
    }
  }
}
