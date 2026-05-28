import type { TTSBackend, SynthOptions } from "../pipeline/interfaces.ts";

export class KokoroBackend implements TTSBackend {
  readonly name = "kokoro";
  constructor(private readonly url: string) {}

  async isAvailable(): Promise<boolean> {
    try {
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(), 1500);
      const r = await fetch(`${this.url}/health`, { signal: ctrl.signal });
      clearTimeout(t);
      return r.ok;
    } catch { return false; }
  }

  async synthesize(text: string, opts: SynthOptions): Promise<Buffer> {
    const resp = await fetch(`${this.url}/v1/audio/speech`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ model: "kokoro", input: text, voice: opts.voice, speed: opts.speed, response_format: "wav" }),
      signal: AbortSignal.timeout(30_000),
    });
    if (!resp.ok) throw new Error(`Kokoro ${resp.status}: ${await resp.text()}`);
    return Buffer.from(await resp.arrayBuffer());
  }
}
