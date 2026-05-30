/**
 * SpeakFacade — unified TTS pipeline.
 *
 * Composes: Translator → TTSBackend → AudioProcessor → Player
 *
 * Concurrency model — two concerns kept separate:
 *
 *   SYNTHESIS  runs in parallel.  Each speak() call kicks off translation +
 *              synthesis + processing immediately, overlapping with whatever
 *              is currently playing.  This hides latency: the next phrase is
 *              ready to play the moment the current one finishes.
 *
 *   PLAYBACK   is serialised via a promise chain (playQueue).  Audio chunks
 *              are played in the order speak() was called, never overlapping.
 *
 *   CANCELLATION via generation counter.  stop() increments the counter.
 *              Any speak() from a previous generation is silently dropped
 *              from the play queue.
 */

import { createHash } from "node:crypto";
import { stripMarkdown } from "../lib.ts";
import type { AudioProcessor, Player, SynthOptions, Translator, TTSBackend } from "./interfaces.ts";

/** Texts shorter than this after markdown stripping are not worth speaking. */
const MIN_SPEAKABLE_LENGTH = 3;

/** How many characters to preview in log output. */
const LOG_PREVIEW_CHARS = 50;

/** Total bytes the audio LRU cache may hold before evicting. */
export const AUDIO_CACHE_MAX_BYTES = 64 * 1024 * 1024; // 64 MB

/** Progress callback passed to SpeakFacade.speak() — receives one-line status strings per synthesis stage. */
export type SpeakProgressCallback = (msg: string) => void;

// ─── Audio LRU cache ──────────────────────────────────────────────────────────

/**
 * In-process LRU cache for synthesised + processed audio buffers.
 *
 * Backed by an ES6 Map whose insertion order gives us LRU semantics:
 *   • get() promotes a hit to the tail (most-recently-used)
 *   • set() evicts from the head (least-recently-used) when over budget
 *
 * Cache key = SHA-1 of (ttsText | backend | voice | speed).
 */
export class AudioLRU {
  private readonly map = new Map<string, Buffer>();
  private _bytes = 0;

  constructor(readonly maxBytes: number = AUDIO_CACHE_MAX_BYTES) {}

  get(key: string): Buffer | undefined {
    const buf = this.map.get(key);
    if (!buf) return undefined;
    this.map.delete(key);
    this.map.set(key, buf);
    return buf;
  }

  set(key: string, buf: Buffer): void {
    // Use get() directly — the result is the source of truth, avoiding a redundant has() check
    const existing = this.map.get(key);
    if (existing !== undefined) {
      this._bytes -= existing.length;
      this.map.delete(key);
    }
    this.map.set(key, buf);
    this._bytes += buf.length;
    for (const [k, v] of this.map) {
      if (this._bytes <= this.maxBytes) break;
      this.map.delete(k);
      this._bytes -= v.length;
    }
  }

  clear(): void { this.map.clear(); this._bytes = 0; }

  get bytes(): number { return this._bytes; }
  get size():  number { return this.map.size; }
}

// ─── SpeakFacade ──────────────────────────────────────────────────────────────

export class SpeakFacade {
  readonly cache: AudioLRU;

  /**
   * Serial playback queue.  Each speak() appends to this chain so audio
   * plays in call order regardless of how many speaks are in-flight.
   */
  private playQueue: Promise<void> = Promise.resolve();

  /**
   * Generation counter for cancellation.
   * stop() increments it; any speak() carrying an older generation is dropped.
   */
  private generation = 0;

  constructor(
    private translator: Translator,
    private backend:    TTSBackend,
    private processor:  AudioProcessor,
    private player:     Player,
    private opts:       SynthOptions,
    cache?:             AudioLRU,
  ) {
    this.cache = cache ?? new AudioLRU();
  }

  get backendName(): string { return this.backend.name; }

  swapTranslator(t: Translator): void     { this.translator = t; }
  swapBackend(b: TTSBackend): void        { this.backend    = b; }
  swapProcessor(p: AudioProcessor): void  { this.processor  = p; }
  setOpts(opts: Partial<SynthOptions>): void { this.opts = { ...this.opts, ...opts }; }

  /** Cancel all pending and in-progress speech. */
  stop(): void {
    this.generation++;
  }

  /** Human-readable cache stats for /tts status. */
  cacheStats(): string {
    const mb = (this.cache.bytes / 1024 / 1024).toFixed(1);
    return `${this.cache.size} entries / ${mb}MB`;
  }

  private buildCacheKey(text: string): string {
    return createHash("sha1")
      .update(`${this.backend.name}|${this.opts.voice}|${this.opts.speed}|${text}`)
      .digest("hex");
  }

  /**
   * Synthesise and enqueue audio for playback.
   *
   * Returns a Promise that resolves when THIS phrase has finished playing
   * (or been cancelled).  Callers may fire-and-forget or await — the play
   * queue is correct either way.
   */
  async speak(rawText: string, log?: SpeakProgressCallback): Promise<void> {
    const emit = log ?? ((_m: string) => {});
    const gen  = this.generation;

    const clean = stripMarkdown(rawText).trim();
    if (clean.length < MIN_SPEAKABLE_LENGTH) {
      emit("skipped: text too short after stripping");
      return;
    }

    // ── Synthesis fires immediately (parallel with current playback) ─────────
    //
    // While phrase N is playing, phrase N+1 is already being translated,
    // synthesised, and processed.  By the time the player is free, the
    // buffer is (usually) ready — zero additional wait.
    const audioPromise: Promise<Buffer | null> = (async () => {
      try {
        const text = await this.translator.translate(clean);
        if (this.generation !== gen) return null;

        emit(`backend=${this.backend.name} text="${text.slice(0, LOG_PREVIEW_CHARS)}…"`);

        const key = this.buildCacheKey(text);
        const hit = this.cache.get(key);
        if (hit) {
          const mb = (this.cache.bytes / 1024 / 1024).toFixed(1);
          emit(`cache hit — ${hit.length} bytes (${this.cache.size} entries, ${mb}MB)`);
          return hit;
        }

        const audio = await this.backend.synthesize(text, this.opts);
        if (this.generation !== gen) return null;
        emit(`synthesized ${audio.length} bytes`);

        const processed = await this.processor.process(audio);
        if (this.generation !== gen) return null;
        if (processed !== audio) emit(`processed via ${this.processor.constructor.name}`);

        this.cache.set(key, processed);
        const mb = (this.cache.bytes / 1024 / 1024).toFixed(1);
        emit(`cached — ${this.cache.size} entries, ${mb}MB`);

        return processed;
      } catch (e: any) {
        emit(`ERROR (synthesis): ${e?.message}`);
        return null;
      }
    })();

    // ── Playback is serialised ────────────────────────────────────────────────
    //
    // We append to the play queue so this phrase plays only after the
    // previous one finishes, regardless of when synthesis completes.
    let resolve!: () => void;
    const played = new Promise<void>(r => { resolve = r; });

    this.playQueue = this.playQueue
      .then(async () => {
        if (this.generation !== gen) { resolve(); return; }

        const audio = await audioPromise;
        if (!audio || this.generation !== gen) { resolve(); return; }

        try {
          await this.player.play(audio);
          emit(`played ${audio.length} bytes`);
        } catch (e: any) {
          emit(`ERROR (playback): ${e?.message}`);
        } finally {
          resolve();
        }
      })
      .catch(() => { resolve(); }); // never break the chain

    return played;
  }
}
