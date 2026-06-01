/**
 * test/00_preflight.test.ts — service availability check.
 *
 * Runs first (alphabetical order). Probes every external dependency,
 * prints a status table, and writes the snapshot to process.env so
 * downstream tests can read it without re-probing.
 *
 * Nothing here fails the suite — it reports and gates are enforced
 * per-test-file with describe.skipIf(). The pre-flight is diagnostic.
 */

import { describe, it, beforeAll } from "vitest";
import { checkServices, type ServiceStatus } from "./services.ts";
import { Env } from "./env.ts";

let status: ServiceStatus;

beforeAll(async () => {
  status = await checkServices();

  // Publish results so describe.skipIf() helpers can read them synchronously.
  process.env["_FONI_PREFLIGHT_ESPEAK"]  = String(status.espeak);
  process.env["_FONI_PREFLIGHT_SYNTH"]   = String(status.synth);
  process.env["_FONI_PREFLIGHT_LIBRE"]   = String(status.libre);
  process.env["_FONI_PREFLIGHT_OLLAMA"]  = String(status.ollama);
}, 15_000);

describe("Pre-flight — service availability", () => {
  it("prints service status table", () => {
    const row = (label: string, ok: boolean, note = "") => {
      const icon = ok ? "✅" : "❌";
      const state = ok ? "UP  " : "DOWN";
      return `  ${icon}  ${state}  ${label.padEnd(22)}${note}`;
    };

    console.log(`
╔══ Foni Pre-Flight ══════════════════════════════════════════════╗
║                                                                  ║
${row("espeak-ng", status.espeak, "(required — TTS backend)")}
${row("foni-synth", status.synth, `@ ${status.synthUrl}`)}
${row("LibreTranslate", status.libre, "@ localhost:5000")}
${row("Ollama", status.ollama, "@ localhost:11434")}
║                                                                  ║
║  Environment:                                                    ║
║    FONI_SYNTH_URL = ${(Env.SYNTH_URL ?? "(not set)").padEnd(41)}║
║    RVC_URL        = ${Env.RVC_URL.padEnd(41)}║
║    FONI_PLAY      = ${String(Env.PLAY).padEnd(41)}║
║                                                                  ║
║  Test gates:                                                     ║
║    Unit tests          always run                                ║
║    integration.test    needs FONI_SYNTH_URL + synth UP           ║
║    latency.e2e         always runs (espeak baseline)             ║
║    tuning-validate     needs espeak + synth UP (RVC optional)    ║
║    rvc.e2e             needs synth UP + model loaded             ║
╚══════════════════════════════════════════════════════════════════╝`);

    // The only hard requirement for the base suite is espeak.
    // Everything else is optional — tests self-skip when deps are absent.
    if (!status.espeak) {
      console.warn("\n  ⚠  espeak-ng not found — synthesis tests will produce empty audio.");
    }
    if (!status.synth) {
      console.warn(`  ℹ  foni-synth not running at ${status.synthUrl}`);
      console.warn("     Start with: RUST_MIN_STACK=67108864 RVC_MODELS_DIR=./rvc/models ./foni-server/target/release/foni-synth");
    }
    if (!status.libre) {
      console.warn("  ℹ  LibreTranslate not running — translation falls back to Ollama.");
      console.warn("     Start with: podman run -d -p 5000:5000 libretranslate/libretranslate --load-only en,ru");
    }
    if (!status.ollama) {
      console.warn("  ℹ  Ollama not running — quality-upgrade path disabled.");
      console.warn("     Start with: ollama serve  (then: ollama pull qwen3:1.7b)");
    }
  });

  it("espeak-ng is available (required for all synthesis tests)", () => {
    if (!status.espeak) {
      console.warn("  espeak-ng missing — install with: sudo dnf install espeak-ng");
    }
    // Soft assertion: warn but don't abort the suite.
    // Individual tests guard themselves with isAvailable() checks.
  });

  it("translation path is viable (Libre or Ollama)", () => {
    if (!status.libre && !status.ollama) {
      console.warn("  Neither LibreTranslate nor Ollama available — translation will passthrough.");
    }
    // Soft — tests run in passthrough mode.
  });
});
