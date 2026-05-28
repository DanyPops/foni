import { spawn, execFileSync } from "node:child_process";
import { platform } from "node:os";
import type { Player } from "./interfaces.ts";

type PlayerBin = "mpv" | "aplay" | "paplay" | "afplay";

function commandExists(cmd: string): boolean {
  try { execFileSync("which", [cmd], { stdio: "ignore" }); return true; }
  catch { return false; }
}

function detectBin(): PlayerBin | null {
  const candidates: PlayerBin[] = platform() === "darwin"
    ? ["mpv", "afplay"]
    : ["mpv", "aplay", "paplay"];
  return candidates.find(commandExists) ?? null;
}

export class SystemPlayer implements Player {
  private bin: PlayerBin | null | undefined;

  detected(): PlayerBin | null {
    if (this.bin === undefined) this.bin = detectBin();
    return this.bin;
  }

  async play(buf: Buffer): Promise<void> {
    const bin = this.detected();
    if (!bin) return;
    const args: Record<PlayerBin, string[]> = {
      mpv:    ["--no-video", "--really-quiet", "--audio-display=no", "-"],
      aplay:  ["-q", "-"],
      paplay: ["-"],
      afplay: ["/dev/stdin"],
    };
    return new Promise<void>((resolve) => {
      const proc = spawn(bin, args[bin], { stdio: ["pipe", "ignore", "ignore"] });
      proc.stdin!.write(buf);
      proc.stdin!.end();
      proc.on("close", () => resolve());
      proc.on("error", () => resolve());
    });
  }
}
