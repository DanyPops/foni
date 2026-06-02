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
  private activeProc: ReturnType<typeof spawn> | null = null;

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
      // args[bin] is always defined: bin is a PlayerBin key and args covers all variants
      const argv = args[bin] ?? [];
      const proc = spawn(bin, argv, { stdio: ["pipe", "ignore", "ignore"] });
      this.activeProc = proc;
      const stdin = proc.stdin;
      if (!stdin) { this.activeProc = null; resolve(); return; }
      stdin.write(buf);
      stdin.end();
      proc.on("close", () => { this.activeProc = null; resolve(); });
      proc.on("error", () => { this.activeProc = null; resolve(); });
    });
  }

  stop(): void {
    if (this.activeProc) {
      this.activeProc.kill("SIGTERM");
      this.activeProc = null;
    }
  }
}
