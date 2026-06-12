/**
 * Modal Job Watcher — injects training state into agent context.
 *
 * On every turn: checks active jobs, appends status to system prompt.
 * On completion/error: triggers agent turn with notification.
 *
 * /modal-watch <call-id>   — start watching
 * /modal-unwatch            — stop
 * /modal-status             — current state
 */

import { execSync } from "node:child_process";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const POLL_MS = 15_000;

interface JobState {
  callId: string;
  lastLog: string;
  step: string;
  status: "running" | "done" | "failed";
  error?: string;
}

export default function (pi: ExtensionAPI) {
  let job: JobState | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;

  function poll(): void {
    if (!job || job.status !== "running") return;
    try {
      const logs = execSync(
        `modal app logs depecher-fish-finetune --function-call ${job.callId} --tail 3 2>&1`,
        { timeout: 8_000, encoding: "utf-8" },
      ).trim();

      if (!logs) return;
      job.lastLog = logs;

      // Parse progress
      const stepMatch = logs.match(/(\d+)\/(\d+)\s*\[.*?loss=([\d.]+).*?accuracy=([\d.]+)/);
      if (stepMatch) {
        job.step = `${stepMatch[1]}/${stepMatch[2]} loss=${stepMatch[3]} acc=${stepMatch[4]}`;
      }

      // Detect completion
      if (logs.includes("[train] DONE")) {
        job.status = "done";
        pi.sendMessage(
          { customType: "modal-watcher", content: `✅ Training complete! Job: ${job.callId}\n${logs}`, display: true },
          { triggerTurn: true },
        );
        return;
      }

      // Detect errors
      if (logs.includes("CalledProcessError") || logs.includes("CUDA out of memory") || logs.includes("crash-looping")) {
        job.status = "failed";
        job.error = logs.split("\n").find(l => l.includes("Error") || l.includes("OOM")) ?? logs;
        pi.sendMessage(
          { customType: "modal-watcher", content: `❌ Training failed: ${job.error}\nJob: ${job.callId}`, display: true },
          { triggerTurn: true },
        );
        return;
      }
    } catch { /* CLI failed, retry next tick */ }
  }

  // Inject job state into every agent turn
  pi.on("before_agent_start", async () => {
    if (!job) return;

    const lines = [
      `\n## Active Modal Training Job`,
      `Call ID: ${job.callId}`,
      `Status: ${job.status}`,
    ];
    if (job.step) lines.push(`Progress: ${job.step}`);
    if (job.error) lines.push(`Error: ${job.error}`);
    if (job.lastLog) lines.push(`Last log: ${job.lastLog.split("\n").pop()}`);

    return {
      message: {
        customType: "modal-job-context",
        content: lines.join("\n"),
        display: false,  // don't clutter TUI, just inject into LLM context
      },
    };
  });

  function stop(): void {
    if (timer) { clearInterval(timer); timer = null; }
    job = null;
  }

  pi.registerCommand("modal-watch", {
    description: "Watch Modal job — /modal-watch <call-id>",
    handler: async (args, ctx) => {
      const id = args.trim();
      if (!id) { ctx.ui.notify("Usage: /modal-watch <call-id>", "warning"); return; }
      stop();
      job = { callId: id, lastLog: "", step: "", status: "running" };
      timer = setInterval(poll, POLL_MS);
      poll();
      ctx.ui.notify(`Watching ${id}\nJob state injected into every turn context.`, "info");
    },
  });

  pi.registerCommand("modal-unwatch", {
    description: "Stop watching",
    handler: async (_args, ctx) => {
      const id = job?.callId;
      stop();
      ctx.ui.notify(id ? `Stopped ${id}` : "Not watching", "info");
    },
  });

  pi.registerCommand("modal-status", {
    description: "Current job state",
    handler: async (_args, ctx) => {
      if (!job) { ctx.ui.notify("Not watching any job", "info"); return; }
      ctx.ui.notify(
        `Job: ${job.callId}\nStatus: ${job.status}\nProgress: ${job.step || "unknown"}\nLast: ${job.lastLog.split("\n").pop() || "none"}`,
        "info",
      );
    },
  });
}
