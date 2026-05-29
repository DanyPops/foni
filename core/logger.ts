/**
 * core/logger.ts — Structured, injectable logger.
 *
 * Level hierarchy: DEBUG < INFO < WARN < ERROR < SILENT
 * Controlled by FONI_LOG_LEVEL env var (default: INFO in production, SILENT in tests).
 * Writes to stderr — never pollutes stdout or test output.
 *
 * Usage:
 *   import { logger } from "../core/logger.ts";
 *   logger.info("pipeline", "RVC latency", { ms: 420 });
 *   logger.warn("ffmpeg", "filter fallback — returning identity", { filter: "loudnorm=..." });
 *
 * In tests: pass a SilentLogger or use withLogger(silent, () => { ... }).
 */

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

function resolveLevel(): LogLevel {
  const env = process.env["FONI_LOG_LEVEL"]?.toUpperCase();
  if (env && env in LEVEL_RANK) return env as LogLevel;
  // Default: silent in vitest, INFO otherwise
  return process.env["VITEST"] ? "SILENT" : "INFO";
}

class StderrLogger implements Logger {
  private readonly minRank: number;

  constructor(level: LogLevel = resolveLevel()) {
    this.minRank = LEVEL_RANK[level];
  }

  private emit(level: LogLevel, component: string, msg: string, data?: Record<string, unknown>): void {
    if (LEVEL_RANK[level] < this.minRank) return;
    const ts   = new Date().toISOString().slice(11, 23); // HH:MM:SS.mmm
    const line = `[${ts}] ${level.padEnd(5)} [${component}] ${msg}`;
    const out  = data && Object.keys(data).length > 0
      ? `${line}  ${JSON.stringify(data)}`
      : line;
    process.stderr.write(out + "\n");
  }

  debug(c: string, m: string, d?: Record<string, unknown>) { this.emit("DEBUG", c, m, d); }
  info (c: string, m: string, d?: Record<string, unknown>) { this.emit("INFO",  c, m, d); }
  warn (c: string, m: string, d?: Record<string, unknown>) { this.emit("WARN",  c, m, d); }
  error(c: string, m: string, d?: Record<string, unknown>) { this.emit("ERROR", c, m, d); }
}

/** No-op logger — zero overhead, no I/O. Used in tests and by default in CI. */
export const silentLogger: Logger = {
  debug() {},
  info () {},
  warn () {},
  error() {},
};

// ─── Global singleton ─────────────────────────────────────────────────────────

/**
 * Module-level logger instance. Respects FONI_LOG_LEVEL env var.
 * Tests run under VITEST=true so this is automatically silent in test runs.
 */
export const logger: Logger = new StderrLogger();

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
