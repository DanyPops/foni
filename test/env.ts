/**
 * test/env.ts — single source of truth for all test environment variables.
 *
 * Import this instead of reading process.env directly in test files.
 * All values are resolved once at module load and are read-only.
 */

export const Env = {
  /** foni-synth base URL. Undefined = server not available. */
  SYNTH_URL:  process.env["FONI_SYNTH_URL"],

  /** Legacy RVC URL (same server, different env var name in older tests). */
  RVC_URL:    process.env["RVC_URL"] ?? process.env["FONI_SYNTH_URL"] ?? "http://127.0.0.1:5050",

  /** Set to "1" to actually play audio during tests. */
  PLAY:       process.env["FONI_PLAY"] === "1",

  /** Set to "1" to make DSP failures throw instead of silently falling back. */
  REQUIRE_DSP: process.env["FONI_REQUIRE_DSP"] === "1",

  /** Default espeak sample rate (Hz). */
  ESPEAK_SR:  22_050,

  /** Default synthesis voice. */
  VOICE:      "ru" as const,

  /** Default synthesis speed (WPM). */
  WPM:        150,
} as const;
