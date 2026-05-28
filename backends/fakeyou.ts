import { buildFakeYouHeaders, FAKEYOU_CDN } from "../lib.ts";
import type { TTSBackend, SynthOptions } from "../pipeline/interfaces.ts";

export class FakeYouBackend implements TTSBackend {
  readonly name = "fakeyou";

  constructor(
    private readonly modelToken: string,
    private readonly apiKey: string = "",
    private readonly pollIntervalMs: number = 2000,
  ) {}

  async isAvailable(): Promise<boolean> {
    if (!this.modelToken) return false;
    try {
      const r = await fetch("https://api.fakeyou.com/tts/list", { signal: AbortSignal.timeout(3000) });
      return r.ok;
    } catch { return false; }
  }

  async synthesize(text: string, _opts: SynthOptions): Promise<Buffer> {
    const headers = buildFakeYouHeaders(this.apiKey);

    const inferResp = await fetch("https://api.fakeyou.com/tts/inference", {
      method: "POST",
      headers,
      body: JSON.stringify({ uuid_idempotency_token: crypto.randomUUID(), tts_model_token: this.modelToken, inference_text: text }),
      signal: AbortSignal.timeout(15_000),
    });
    if (!inferResp.ok) throw new Error(`FakeYou inference ${inferResp.status}: ${await inferResp.text()}`);
    const { inference_job_token } = await inferResp.json() as { inference_job_token: string };

    const pollStart = Date.now();
    let wavPath: string | null = null;
    for (let i = 0; i < 60; i++) {
      await new Promise<void>(r => setTimeout(r, this.pollIntervalMs));
      const poll = await fetch(`https://api.fakeyou.com/tts/job/${inference_job_token}`, { headers, signal: AbortSignal.timeout(10_000) });
      if (!poll.ok) continue;
      const { state } = await poll.json() as { state: { status: string; maybe_public_bucket_wav_audio_path: string | null } };
      if (state.status === "complete_success" && state.maybe_public_bucket_wav_audio_path) {
        wavPath = state.maybe_public_bucket_wav_audio_path;
        console.debug(`[foni] FakeYou ready after ${i + 1} poll(s), ${Date.now() - pollStart}ms`);
        break;
      }
      if (state.status === "complete_failure" || state.status === "dead") throw new Error(`FakeYou job failed: ${state.status}`);
    }
    if (!wavPath) throw new Error("FakeYou job timed out");

    const audioResp = await fetch(`${FAKEYOU_CDN}${wavPath}`, { signal: AbortSignal.timeout(30_000) });
    if (!audioResp.ok) throw new Error(`FakeYou CDN ${audioResp.status}`);
    return Buffer.from(await audioResp.arrayBuffer());
  }
}
