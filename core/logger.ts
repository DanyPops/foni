/**
 * core/logger.ts — Structured, injectable logger.
 *
 * Level hierarchy: DEBUG < INFO < WARN < ERROR < SILENT
 *
 * DEFAULT: SILENT — extensions must never write to stdout/stderr,
 * which belong to Pi's TUI. See web-spider's own pattern:
 *   "Diagnostics go only to a file — never to stdout/stderr."
 *
 * To get logs, set FONI_LOG_LEVEL and FONI_LOG_PATH:
 *   FONI_LOG_LEVEL=INFO FONI_LOG_PATH=~/.cache/foni/foni.log pi
 *
 * Or for one-shot CLI use:
 *   FONI_LOG_LEVEL=INFO npx tsx scripts/gap-report.mts
 *   (CLI scripts write to stderr directly — no Pi TUI to corrupt.)
 */

import { appendFileSync, mkdirSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { homedir } from "node:os";

// ─── Level ────────────────────────────────────────────────────────────────────

export type LogLevel = "DEBUG" | "INFO" | "WARN" | "ERROR" | "SILENT";

const LEVEL_RANK: Record<LogLevel, number> = {
  DEBUG:  0,
  INFO:   1,
  WARN:   2,
  ERROR:  3,
  SILENT: 4,
};

// ─── Interface ────────────────────────────────────────────────────────────────

export interface Logger {
  debug(component: string, msg: string, data?: Record<string, unknown>): void;
  info (component: string, msg: string, data?: Record<string, unknown>): void;
  warn (component: string, msg: string, data?: Record<string, unknown>): void;
  error(component: string, msg: string, data?: Record<string, unknown>): void;
}

// ─── Implementations ──────────────────────────────────────────────────────────

/** No-op logger — zero overhead, no I/O. The module default. */
export const silentLogger: Logger = {
  debug() {},
  info () {},
  warn () {},
  error() {},
};

function resolveLevel(): LogLevel {
  const env = process.env["FONI_LOG_LEVEL"]?.toUpperCase();
  if (env && env in LEVEL_RANK) return env as LogLevel;
  // Default: SILENT everywhere — extensions must never leak to Pi's TUI.
  // VITEST check kept for safety; the real guard is the default being SILENT.
  return "SILENT";
}

function resolveLogPath(): string | null {
  return process.env["FONI_LOG_PATH"]
    ?? (process.env["FONI_LOG_LEVEL"] && !process.env["VITEST"]
      // When a level is set but no path, write to a default log file.
      // Never stderr — that belongs to Pi's TUI.
      ? join(homedir(), ".cache", "foni", "foni.log")
      : null);
}

/**
 * File-based logger. Writes to FONI_LOG_PATH (or ~/.cache/foni/foni.log).
 * Never touches stdout/stderr so Pi's TUI is not corrupted.
 */
class FileLogger implements Logger {
  private readonly minRank: number;
  private readonly path:    string;
  private ready = false;

  constructor(level: LogLevel, path: string) {
    this.minRank = LEVEL_RANK[level];
    this.path    = path;
  }

  private ensureDir(): void {
    if (this.ready) return;
    try {
      const dir = dirname(this.path);
      if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
      this.ready = true;
    } catch { /* best-effort — if we can't create the dir, just skip */ }
  }

  private emit(level: LogLevel, component: string, msg: string, data?: Record<string, unknown>): void {
    if (LEVEL_RANK[level] < this.minRank) return;
    this.ensureDir();
    const ts   = new Date().toISOString().slice(11, 23);
    const line = `[${ts}] ${level.padEnd(5)} [${component}] ${msg}`;
    const out  = data && Object.keys(data).length > 0
      ? `${line}  ${JSON.stringify(data)}`
      : line;
    try { appendFileSync(this.path, out + "\n"); } catch { /* best-effort */ }
  }

  debug(c: string, m: string, d?: Record<string, unknown>) { this.emit("DEBUG", c, m, d); }
  info (c: string, m: string, d?: Record<string, unknown>) { this.emit("INFO",  c, m, d); }
  warn (c: string, m: string, d?: Record<string, unknown>) { this.emit("WARN",  c, m, d); }
  error(c: string, m: string, d?: Record<string, unknown>) { this.emit("ERROR", c, m, d); }
}

// ─── Global singleton ─────────────────────────────────────────────────────────

function buildLogger(): Logger {
  const level = resolveLevel();
  if (level === "SILENT") return silentLogger;
  const path = resolveLogPath();
  if (!path) return silentLogger;
  return new FileLogger(level, path);
}

export const logger: Logger = buildLogger();

// ─── Injectable context helper ────────────────────────────────────────────────

let _active: Logger = logger;

/** Swap the active logger for the duration of `fn`. Safe for concurrent use. */
export function withLogger<T>(log: Logger, fn: () => T): T {
  const prev = _active;
  _active = log;
  try { return fn(); }
  finally { _active = prev; }
}

/** The currently active logger — use this in pipeline code for testability. */
export function getLogger(): Logger { return _active; }
