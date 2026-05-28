import type { AudioProcessor } from "./interfaces.ts";

export class IdentityProcessor implements AudioProcessor {
  async process(input: Buffer): Promise<Buffer> {
    return input;
  }
}

export class RVCProcessor implements AudioProcessor {
  constructor(private readonly url: string, private readonly timeoutMs = 60_000) {}

  async process(input: Buffer): Promise<Buffer> {
    try {
      const resp = await fetch(`${this.url}/convert`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ audio_data: input.toString("base64") }),
        signal: AbortSignal.timeout(this.timeoutMs),
      });
      if (!resp.ok) return input;
      return Buffer.from(await resp.arrayBuffer());
    } catch {
      return input;
    }
  }
}
