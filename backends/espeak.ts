import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, readFileSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { TTSBackend, SynthOptions } from "../pipeline/interfaces.ts";

/**
 * espeak-ng native speaking rate at speed=1.0.
 * The actual rate passed to espeak is `BASE_WPM * opts.speed`.
 * Increasing this makes the baseline voice faster; decreasing slows it down.
 */
export const ESPEAK_BASE_WPM = 160;

export class EspeakBackend implements TTSBackend {
  readonly name = "espeak";

  constructor(private readonly lang: string = "en") {}

  isAvailable(): Promise<boolean> {
    try { execFileSync("which", ["espeak-ng"], { stdio: "ignore" }); return Promise.resolve(true); }
    catch { return Promise.resolve(false); }
  }

  async synthesize(text: string, opts: SynthOptions): Promise<Buffer> {
    const rate = Math.round(ESPEAK_BASE_WPM * opts.speed);
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
