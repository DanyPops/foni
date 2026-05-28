import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, readFileSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { TTSBackend, SynthOptions } from "../pipeline/interfaces.ts";

export class EspeakBackend implements TTSBackend {
  readonly name = "espeak";

  constructor(private readonly lang: string = "en") {}

  isAvailable(): Promise<boolean> {
    try { execFileSync("which", ["espeak-ng"], { stdio: "ignore" }); return Promise.resolve(true); }
    catch { return Promise.resolve(false); }
  }

  async synthesize(text: string, opts: SynthOptions): Promise<Buffer> {
    const rate = Math.round(160 * opts.speed);
    const outPath = join(tmpdir(), `foni-espeak-${Date.now()}.wav`);
    await new Promise<void>((resolve, reject) => {
      const proc = spawn("espeak-ng", ["-s", String(rate), "-v", this.lang, "-w", outPath, text], { stdio: "ignore" });
      proc.on("close", (code) => code === 0 ? resolve() : reject(new Error(`espeak-ng exited ${code}`)));
      proc.on("error", reject);
    });
    const buf = Buffer.from(readFileSync(outPath));
    try { unlinkSync(outPath); } catch { /* best-effort cleanup */ }
    return buf;
  }
}
