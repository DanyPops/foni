/**
 * pipeline/synth-backend.ts — HTTP client for Rust /synthesize.
 *
 * Replaces the three-class chain:
 *   ProsodyBackend(EspeakBackend) + RVCProcessor + SmoothingProcessor
 *
 * A single POST /synthesize call handles:
 *   SSML annotation → espeak → ContentVec → RMVPE → Generator → DSP → WAV
 *
 * Falls back to null (no audio) when foni-synth is unreachable, so the
 * engine degrades gracefully rather than erroring.
 */

import type { TTSBackend, SynthOptions } from "./interfaces.ts";
import { getLogger } from "../core/logger.ts";

export interface SynthBackendOpts {
  /** Base URL of foni-synth.  Default: http://localhost:5050 */
  url:      string;
  /** RVC model name.  Default: "bandit" */
  model?:   string;
  /** Apply per-sentence SSML prosody variation (rate/pitch/range). Default: true */
  prosody?: boolean;
  /** Global rate override % — ignored when prosody=true. */
  ratePct?: number;
  /** Global pitch override (espeak 0-99 scale, 50=normal). */
  pitchPt?: number;
  /** Global range override: "x-high" | "high" | "medium" | "low". */
  range?:   string;
}

const log = getLogger();

export class SynthBackend implements TTSBackend {
  readonly name = "synth";

  constructor(private readonly opts: SynthBackendOpts) {}

  async isAvailable(): Promise<boolean> {
    try {
      const r = await fetch(`${this.opts.url}/params`, {
        signal: AbortSignal.timeout(2_000),
      });
      return r.ok;
    } catch { return false; }
  }

  async synthesize(text: string, voiceOpts: SynthOptions): Promise<Buffer> {
    const body: Record<string, unknown> = {
      text,
      voice:   voiceOpts.voice ?? "ru",
      speed:   voiceOpts.speed ? Math.round(voiceOpts.speed * 130) : 150,
      model:   this.opts.model ?? "bandit",
      prosody: this.opts.prosody ?? true,
      dsp:     true,
    };

    if (this.opts.ratePct !== undefined) body["rate_pct"] = this.opts.ratePct;
    if (this.opts.pitchPt !== undefined) body["pitch_pt"] = this.opts.pitchPt;
    if (this.opts.range   !== undefined) body["range"]    = this.opts.range;

    log.debug("SynthBackend", "POST /synthesize", {
      preview: text.slice(0, 40),
      url:     this.opts.url,
    });

    const t0   = Date.now();
    const resp = await fetch(`${this.opts.url}/synthesize`, {
      method:  "POST",
      headers: { "Content-Type": "application/json" },
      body:    JSON.stringify(body),
      signal:  AbortSignal.timeout(120_000),
    });

    if (!resp.ok) {
      const msg = await resp.text().catch(() => resp.statusText);
      throw new Error(`/synthesize HTTP ${resp.status}: ${msg}`);
    }

    const wav = Buffer.from(await resp.arrayBuffer());
    log.info("SynthBackend", "synthesize OK", {
      ms:    Date.now() - t0,
      bytes: wav.length,
    });
    return wav;
  }
}
