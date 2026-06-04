/**
 * test/preflight.ts — vitest globalSetup.
 *
 * Runs once before any test file. Probes every external dependency,
 * prints a status table to stdout, and exports results via teardown
 * so tests can gate on `process.env._FONI_*` flags set here.
 *
 * Registered in vitest.config.ts as globalSetup.
 */

import { checkServices } from "./services.ts";
import { Env }           from "./env.ts";

export async function setup(): Promise<void> {
  const s = await checkServices();

  // Publish for per-test skipIf guards (process is shared in same worker).
  process.env["_FONI_PREFLIGHT_ESPEAK"]  = String(s.espeak);
  process.env["_FONI_PREFLIGHT_SYNTH"]   = String(s.synth);
  process.env["_FONI_PREFLIGHT_LIBRE"]   = String(s.libre);
  process.env["_FONI_PREFLIGHT_OLLAMA"]  = String(s.ollama);

  const row = (label: string, ok: boolean, note = "") =>
    `  ${ok ? "✅" : "❌"}  ${ok ? "UP  " : "DOWN"}  ${label.padEnd(22)}${note}`;

  console.log(`
╔══ Foni Pre-Flight ══════════════════════════════════════════════╗
║                                                                  ║
${row("espeak-ng",     s.espeak, "(required — TTS backend)")}
${row("foni-synth",   s.synth,  `@ ${s.synthUrl}`)}
${row("LibreTranslate", s.libre, "@ localhost:5000")}
${row("Ollama",       s.ollama, "@ localhost:11434")}
║                                                                  ║
║  Environment:                                                    ║
║    FONI_SYNTH_URL = ${(Env.SYNTH_URL ?? "(not set)").padEnd(41)}║
║    RVC_URL        = ${Env.RVC_URL.padEnd(41)}║
║    FONI_PLAY      = ${String(Env.PLAY).padEnd(41)}║
║                                                                  ║
║  Test gates:                                                     ║
║    Unit tests        always run                                  ║
║    integration.test  needs FONI_SYNTH_URL + synth UP             ║
║    latency.e2e       always runs (espeak baseline)               ║
║    tuning-validate   needs espeak + synth UP                     ║
║    rvc.e2e           needs synth UP + model loaded               ║
╚══════════════════════════════════════════════════════════════════╝`);

  if (!s.espeak)
    console.warn("  ⚠  espeak-ng not found — synthesis tests will produce empty audio.\n     sudo dnf install espeak-ng");
  if (!s.synth)
    console.warn(`  ℹ  foni-synth not running at ${s.synthUrl}\n     RUST_MIN_STACK=67108864 RVC_MODELS_DIR=./training/models ./foni-server/target/release/foni-synth`);
  if (!s.libre)
    console.warn("  ℹ  LibreTranslate down — translation falls back to Ollama.\n     podman run -d -p 5000:5000 libretranslate/libretranslate --load-only en,ru");
  if (!s.ollama)
    console.warn("  ℹ  Ollama not running — quality-upgrade path disabled.\n     ollama serve && ollama pull qwen3:1.7b");

  console.log(); // blank line before first test file output
}
