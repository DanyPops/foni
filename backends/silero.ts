import type { TTSBackend, SynthOptions } from "../pipeline/interfaces.ts";

export class SileroBackend implements TTSBackend {
  readonly name = "silero";
  constructor(private readonly url: string) {}

  async isAvailable(): Promise<boolean> {
    try {
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(), 1500);
      const r = await fetch(`${this.url}/tts/speakers`, { signal: ctrl.signal });
      clearTimeout(t);
      return r.ok;
    } catch { return false; }
  }

  async synthesize(text: string, opts: SynthOptions): Promise<Buffer> {
    const resp = await fetch(`${this.url}/tts/generate`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ speaker: opts.voice, text }),
      signal: AbortSignal.timeout(30_000),
    });
    if (!resp.ok) throw new Error(`Silero ${resp.status}: ${await resp.text()}`);
    return Buffer.from(await resp.arrayBuffer());
  }
}
