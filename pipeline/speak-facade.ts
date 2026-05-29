/**
 * SpeakFacade — unified TTS pipeline.
 *
 * Composes: Translator → TTSBackend → AudioProcessor → Player
 *
 * Callers interact only with this facade. They never touch individual
 * backends, processors, or the player directly.
 */

import { createHash } from "node:crypto";
import { stripMarkdown } from "../lib.ts";
import type { AudioProcessor, Player, SynthOptions, Translator, TTSBackend } from "./interfaces.ts";

/** Texts shorter than this after markdown stripping are not worth speaking. */
const MIN_SPEAKABLE_LENGTH = 3;

/** How many characters to preview in log output. */
const LOG_PREVIEW_CHARS = 50;

/** Total bytes the audio LRU cache may hold before evicting. */
const AUDIO_CACHE_MAX_BYTES = 64 * 1024 * 1024; // 64 MB

export type Log = (msg: string) => void;

// ─── Audio LRU cache ───────────────────────────────────────────────────────────────

/**
 * In-process LRU cache for synthesised + processed audio buffers.
 *
 * Backed by an ES6 Map whose insertion order gives us LRU semantics:
 *   • get() promotes a hit to the tail (most-recently-used)
 *   • set() evicts from the head (least-recently-used) when over budget
 *
 * Cache key = SHA-1 of (ttsText | backend | voice | speed).
 * Mat/interject mutations are included because they run BEFORE translation
 * output reaches the cache — the cached text is already the mutated form.
 */
export class AudioLRU {
  private readonly map = new Map<string, Buffer>();
  private _bytes = 0;

  constructor(readonly maxBytes: number = AUDIO_CACHE_MAX_BYTES) {}

  get(key: string): Buffer | undefined {
    const buf = this.map.get(key);
    if (!buf) return undefined;
    // Promote to tail (most-recently-used)
    this.map.delete(key);
    this.map.set(key, buf);
    return buf;
  }

  set(key: string, buf: Buffer): void {
    if (this.map.has(key)) {
      this._bytes -= this.map.get(key)!.length;
      this.map.delete(key);
    }
    this.map.set(key, buf);
    this._bytes += buf.length;
    // Evict from head (least-recently-used) until under limit
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

export class SpeakFacade {
  readonly cache: AudioLRU;

  constructor(
    private translator: Translator,
    private backend: TTSBackend,
    private processor: AudioProcessor,
    private player: Player,
    private opts: SynthOptions,
    cache?: AudioLRU,
  ) {
    this.cache = cache ?? new AudioLRU();
  }

  get backendName(): string { return this.backend.name; }

  swapTranslator(t: Translator): void { this.translator = t; }
  swapBackend(b: TTSBackend): void    { this.backend = b; }
  swapProcessor(p: AudioProcessor): void { this.processor = p; }
  setOpts(opts: Partial<SynthOptions>): void { this.opts = { ...this.opts, ...opts }; }

  /** Stats string for status bar / /tts status. */
  cacheStats(): string {
    const mb = (this.cache.bytes / 1024 / 1024).toFixed(1);
    return `${this.cache.size} entries / ${mb}MB`;
  }

  private buildCacheKey(text: string): string {
    return createHash("sha1")
      .update(`${this.backend.name}|${this.opts.voice}|${this.opts.speed}|${text}`)
      .digest("hex");
  }

  async speak(rawText: string, log?: Log): Promise<void> {
    const emit = log ?? ((_m: string) => {});

    const clean = stripMarkdown(rawText).trim();
    if (clean.length < MIN_SPEAKABLE_LENGTH) { emit("skipped: text too short after stripping"); return; }

    const text = await this.translator.translate(clean);
    const key  = this.buildCacheKey(text);

    // ─ Cache hit: skip synthesis + processing entirely ────────────────────
    const hit = this.cache.get(key);
    if (hit) {
      const mb = (this.cache.bytes / 1024 / 1024).toFixed(1);
      emit(`cache hit — ${hit.length} bytes (${this.cache.size} entries, ${mb}MB used)`);
      await this.player.play(hit);
      return;
    }

    // ─ Cache miss: full pipeline ───────────────────────────────────
    emit(`backend=${this.backend.name} text="${text.slice(0, LOG_PREVIEW_CHARS)}…"`);

    try {
      const audio = await this.backend.synthesize(text, this.opts);
      emit(`synthesized ${audio.length} bytes`);

      const processed = await this.processor.process(audio);
      if (processed !== audio) emit(`processed via ${this.processor.constructor.name}`);

      this.cache.set(key, processed);
      const mb = (this.cache.bytes / 1024 / 1024).toFixed(1);
      emit(`cached — ${this.cache.size} entries, ${mb}MB`);

      await this.player.play(processed);
      emit(`played ${processed.length} bytes`);
    } catch (e: any) {
      emit(`ERROR: ${e?.message}`);
    }
  }
}
