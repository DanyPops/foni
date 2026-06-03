// ─── FoniEngine ───────────────────────────────────────────────────────────────
//
// Domain object that owns the full TTS pipeline.
// Zero pi/ExtensionAPI imports — usable standalone, testable without a pi session.
//
// The pi extension (index.ts) is a thin adapter that:
//   1. Calls engine.onDelta() on message_update events
//   2. Calls engine.onMessageEnd() on message_end
//   3. Calls engine.reset() on agent_start
//   4. Calls engine.prewarm() on session_start
//   5. Reads engine.status() to update the status bar
//   6. Routes /tts commands to engine mutators



import type { FoniConfig }    from "./config.ts";
import { PREWARM_RU, FILLER_PHRASES } from "./config.ts";
import { freshState, resolveBacktickRun, drainChunks } from "./stream.ts";
import type { StreamState }   from "./stream.ts";
import {
  detectEmotion, updateEmotionState, effectiveWeights, neutralState, currentIntensity,
  EMOTION_EMOJI,
} from "./emotion.ts";
import type { EmotionState } from "./emotion.ts";

import type {
  FacadePort, FacadeFactory, TranslatorFactory, ProcessorFactory, AudioProcessor,
} from "./interfaces.ts";

// ─── Re-export factory types for the adapter layer (index.ts) ─────────────────
export type { FacadeFactory, TranslatorFactory, ProcessorFactory, AudioProcessor };

// ─── Status snapshot (read by extension for status bar) ──────────────────────

export interface EngineStatus {
  enabled:          boolean;
  backendName:      string;
  rvcModel:         string | null;
  inputLang:        "en" | "ru";
  outputLang:       "en" | "ru";
  matEnabled:       boolean;
  interjectEnabled: boolean;
  emotionEmoji:     string;   // "" when neutral or intensity < 0.3
  emotionSignals:   string[]; // triggered heuristics for debug
}

// ─── Engine ───────────────────────────────────────────────────────────────────

export class FoniEngine {
  private facade: FacadePort | null = null;
  private facadePromise: Promise<FacadePort | null> | null = null;
  private audioQueue: Promise<void>  = Promise.resolve();
  private streamState: StreamState   = freshState();
  private emotionState: EmotionState = neutralState();
  private fillerCache: Buffer[]      = [];
  private fillerActive               = false;

  /**
   * FoniEngine receives abstract factories rather than constructing backends
   * and processors itself. This satisfies the Dependency Inversion Principle
   * and breaks the core↔pipeline↔backends import cycle.
   *
   * The adapter layer (index.ts) supplies concrete implementations.
   */
  constructor(
    public readonly config:              FoniConfig,
    private readonly facadeFactory:      FacadeFactory,
    private readonly translatorFactory:  TranslatorFactory,
    private readonly processorFactory:   ProcessorFactory,
  ) {}

  // ── Pipeline assembly ───────────────────────────────────────────────────────

  async buildFacade(): Promise<FacadePort | null> {
    const translator = this.translatorFactory(this.config, this.emotionState);
    return this.facadeFactory(this.config, translator, this.emotionState);
  }

  async ensureFacade(): Promise<FacadePort | null> {
    if (this.facade) return this.facade;
    if (!this.facadePromise) this.facadePromise = this.buildFacade();
    const facade = await this.facadePromise;
    if (!facade) this.facadePromise = null; // allow retry when no backend is available
    else this.facade = facade;
    return facade;
  }

  rebuildTranslator(): void {
    this.facade?.swapTranslator(
      this.translatorFactory(this.config, this.emotionState),
    );
  }

  /**
   * Hot-swap the audio processor chain.
   * The adapter layer (index.ts) constructs and passes the pre-built processor
   * — engine.ts never touches concrete processor classes.
   */
  swapProcessor(p: AudioProcessor): void {
    this.facade?.swapProcessor(p);
  }

  invalidateFacade(): void { this.facade = null; this.facadePromise = null; }

  // ── Play queue ──────────────────────────────────────────────────────────────

  muted = false;

  mute(): void   { this.muted = true;  this.stop(); }
  unmute(): void { this.muted = false; }

  enqueue(text: string): void {
    if (this.muted) return;
    this.audioQueue = this.audioQueue.then(async () => {
      const f = await this.ensureFacade();
      if (f) await f.speak(text);
    });
  }

  stop(): void {
    this.audioQueue = Promise.resolve();
    this.stopFiller();
  }

  // ── Filler sounds ───────────────────────────────────────────────────────────

  fillerCount(): number { return this.fillerCache.length; }

  startFiller(): void {
    if (!this.config.enabled || this.muted || this.fillerCache.length === 0 || !this.facade) return;
    const idx = Math.floor(Math.random() * this.fillerCache.length);
    this.fillerActive = true;
    this.facade.playFiller(this.fillerCache[idx]);
  }

  stopFiller(): void {
    if (!this.fillerActive) return;
    this.fillerActive = false;
    this.facade?.stopFiller();
  }

  // ── Stream delta processing ─────────────────────────────────────────────────

  /** Feed one streaming text delta from the LLM. Enqueues complete chunks. */
  onDelta(delta: string): void {
    if (!this.config.enabled || !delta) return;
    if (this.fillerActive) this.stopFiller();
    for (const ch of delta) {
      if (ch === "`") {
        this.streamState.backtickRun++;
      } else {
        if (this.streamState.backtickRun > 0) resolveBacktickRun(this.streamState);
        if (this.streamState.codeDepth === 0 && !this.streamState.inInlineCode) {
          this.streamState.buffer += ch;
        }
      }
    }
    const { chunks, remainder } = drainChunks(this.streamState.buffer);
    this.streamState.buffer = remainder;
    for (const chunk of chunks) this.enqueue(chunk);
  }

  /** Called when the assistant message is complete. Flushes remaining buffer. */
  onMessageEnd(): void {
    if (!this.config.enabled) return;
    const leftover = this.streamState.buffer.trim();
    this.streamState = freshState();
    if (leftover.length > 2) this.enqueue(leftover);
  }

  /**
   * Process a user message — detect emotion, update decay state, rebuild translator.
   * Call from the pi adapter on message_end with role==='user'.
   */
  onUserMessage(text: string): void {
    const reading = detectEmotion(text);
    this.emotionState = updateEmotionState(this.emotionState, reading);
    // Rebuild translator so next audio chunk uses updated effective weights
    this.rebuildTranslator();
  }

  /** Reset stream state and stop audio (call on agent_start). */
  reset(): void {
    this.streamState  = freshState();
    this.emotionState = neutralState();
    this.stop();
  }

  // ── Session prewarm ─────────────────────────────────────────────────────────

  /** Fire-and-forget parallel synthesis of common phrases into AudioLRU. */
  async prewarm(): Promise<void> {
    if (!this.config.rvcEnabled || this.config.outputLang !== "ru") return;
    const f = await this.ensureFacade();
    if (!f) return;
    await Promise.all(PREWARM_RU.map(p => f.synthesizeOnly(p).catch(() => {})));
    await this.prewarmFillers(f);
  }

  private async prewarmFillers(f: FacadePort): Promise<void> {
    const results = await Promise.allSettled(
      FILLER_PHRASES.map(text => f.synthesizeRaw(text)),
    );
    this.fillerCache = results
      .filter((r): r is PromiseFulfilledResult<Buffer | null> => r.status === "fulfilled")
      .map(r => r.value)
      .filter((b): b is Buffer => b != null);
  }

  // ── Observability ───────────────────────────────────────────────────────────

  cacheStats(): string {
    return this.facade?.cacheStats() ?? "not built";
  }

  clearCache(): void {
    this.facade?.cache.clear();
  }

  status(): EngineStatus {
    const intensity = currentIntensity(this.emotionState);
    return {
      enabled:          this.config.enabled,
      backendName:      this.facade?.backendName ?? "...",
      rvcModel:         this.config.rvcEnabled && this.config.rvcModel ? this.config.rvcModel : null,
      inputLang:        this.config.inputLang,
      outputLang:       this.config.outputLang,
      matEnabled:       this.config.matEnabled,
      interjectEnabled: this.config.interjectEnabled,
      emotionEmoji:     intensity >= 0.3 ? EMOTION_EMOJI[this.emotionState.emotion] : "",
      emotionSignals:   [], // populated by onUserMessage, stored separately if needed
    };
  }
}
