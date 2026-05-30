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
import { PREWARM_RU }         from "./config.ts";
import { freshState, resolveBacktickRun, drainChunks } from "./stream.ts";
import type { StreamState }   from "./stream.ts";
import {
  detectEmotion, updateEmotionState, effectiveWeights, neutralState, currentIntensity,
  EMOTION_EMOJI,
} from "./emotion.ts";
import type { EmotionState } from "./emotion.ts";

import { SpeakFacade } from "../pipeline/speak-facade.ts";
import {
  PipelineTranslator,
  makeTranslateMiddleware,
  makeMatMiddleware,
  makeInterjectMiddleware,
  makeITGlossaryMiddleware,
  type TextMiddleware,
} from "../pipeline/translators.ts";
import type { TTSBackend, AudioProcessor, Player } from "../pipeline/interfaces.ts";

import { BIAS_WORDS } from "./emotion.ts";

// ─── Dependency-injection types ───────────────────────────────────────────────
//
// FoniEngine receives factories from the adapter layer (index.ts).
// This keeps core/ free of concrete backend/processor/player imports.
// Dependency Inversion Principle: depend on abstractions, not implementations.

/** Detects availability and constructs the appropriate TTSBackend. */
export type BackendFactory = (config: FoniConfig) => Promise<TTSBackend | null>;

/** Constructs the AudioProcessor chain (RVC, smoothing, breath injection). */
export type ProcessorFactory = (config: FoniConfig) => AudioProcessor;

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
  private facade: SpeakFacade | null = null;
  private audioQueue: Promise<void>  = Promise.resolve();
  private streamState: StreamState   = freshState();
  private emotionState: EmotionState = neutralState();

  /**
   * FoniEngine receives abstract factories rather than constructing backends
   * and processors itself. This satisfies the Dependency Inversion Principle
   * and breaks the core↔pipeline↔backends import cycle.
   *
   * The adapter layer (index.ts) supplies concrete implementations.
   */
  constructor(
    public readonly config:            FoniConfig,
    private readonly backendFactory:   BackendFactory,
    private readonly processorFactory: ProcessorFactory,
    private readonly player:           Player,
  ) {}

  // ── Pipeline assembly ───────────────────────────────────────────────────────

  private buildPipeline(): TextMiddleware[] {
    const stack: TextMiddleware[] = [];
    stack.push(makeITGlossaryMiddleware());
    if (this.config.inputLang !== this.config.outputLang) {
      stack.push(makeTranslateMiddleware(this.config.inputLang, this.config.outputLang));
    }
    if (this.config.outputLang === "ru") {
      // Apply emotion multipliers — decayed lazily at build time
      const ew = effectiveWeights(this.emotionState);
      const matProb       = Math.min(1, this.config.matProb       * ew.matMultiplier);
      const interjectProb = Math.min(1, this.config.interjectProb * ew.interjectMultiplier);
      const bias = ew.wordBias;
      // BIAS_WORDS injected here — not imported in translators.ts (breaks pipeline→core cycle)
      if (this.config.matEnabled)       stack.push(makeMatMiddleware(matProb, this.config.matStretch, bias, BIAS_WORDS));
      if (this.config.interjectEnabled) stack.push(makeInterjectMiddleware(interjectProb, bias, BIAS_WORDS));
    }
    return stack;
  }

  async buildFacade(): Promise<SpeakFacade | null> {
    // Concrete construction fully delegated to injected factories (DIP).
    const backend = await this.backendFactory(this.config);
    if (!backend) return null;

    const translator = new PipelineTranslator(this.buildPipeline(), this.config.outputLang);
    const processor  = this.processorFactory(this.config);

    return new SpeakFacade(translator, backend, processor, this.player, {
      voice: this.config.voice,
      speed: this.config.speed,
    });
  }

  async ensureFacade(): Promise<SpeakFacade | null> {
    if (!this.facade) this.facade = await this.buildFacade();
    return this.facade;
  }

  rebuildTranslator(): void {
    this.facade?.swapTranslator(
      new PipelineTranslator(this.buildPipeline(), this.config.outputLang),
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

  invalidateFacade(): void { this.facade = null; }

  // ── Play queue ──────────────────────────────────────────────────────────────

  enqueue(text: string): void {
    this.audioQueue = this.audioQueue.then(async () => {
      const f = await this.ensureFacade();
      if (f) await f.speak(text);
    });
  }

  stop(): void { this.audioQueue = Promise.resolve(); }

  // ── Stream delta processing ─────────────────────────────────────────────────

  /** Feed one streaming text delta from the LLM. Enqueues complete chunks. */
  onDelta(delta: string): void {
    if (!this.config.enabled || !delta) return;
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
    await Promise.all(PREWARM_RU.map(p => f.speak(p).catch(() => {})));
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
