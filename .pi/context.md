# Foni — project context

## What this is
Foni is a pi TTS extension that synthesizes Russian speech for an AI assistant (Sidorovich character from S.T.A.L.K.E.R.).

## Stack
- **TypeScript** (adapter layer only): `index.ts`, `tui/`, `pipeline/`
- **Rust** (engine): `depecher-server/` — Cargo workspace with three crates:
  - `depecherd` — axum HTTP server, ONNX inference, DSP chain
  - `depecher-analyse` — audio metrics library (MCD, WER, pitch, gap)
  - `depecher-cli` — `depecherctl` CLI tool

## Running depecherd
```bash
RUST_MIN_STACK=67108864 \
RVC_MODELS_DIR=/home/dpopsuev/Projects/foni/rvc/models \
./depecher-server/target/release/depecherd
# defaults to 0.0.0.0:5050 (configurable via DEPECHER_SYNTH_ADDR or rvc/foni-rvc.yaml)
```

## depecherctl
```bash
export DEPECHER_SYNTH_URL=http://localhost:5050
depecherctl status                          # health check
depecherctl synth "текст" --play            # synthesize & play
depecherctl studio "текст"                  # maquette A/B comparison loop
depecherctl samples --out-dir samples/      # batch generate
depecherctl analyse samples/foo.wav --vs baseline/stalker/wav/sidorovich/trader1a.wav
```

## Key paths
- `rvc/models/bandit/` — bandit voice model + `onnx/generator.onnx`
- `rvc/models/pretrained/` — shared ONNX models (contentvec, rmvpe)
- `rvc/foni-rvc.yaml` — server config (model, f0up_key, etc.)
- `baseline/stalker/wav/sidorovich/trader1a.wav` — reference audio
- `samples/` — generated comparison samples

## Quality gate
```bash
# TypeScript
npx tsc --noEmit && npx vitest run --exclude "**/*.e2e*"

# Rust (from depecher-server/)
cargo fmt --all
cargo clippy --workspace --all-targets   # zero warnings
cargo test --workspace
```

## Scribe
- Scope: `foni`, Campaign: `FON-CMP-2`
- Active goals: `FON-GOL-8` (acoustic gap <15%), `FON-GOL-10` (complete — strangler fig done)

## Architecture
- Zero Python at runtime — all inference is Rust via ort (ONNX Runtime)
- Zero ffmpeg — DSP is pure Rust (depecherd `/process`)
- Zero TS analysis code — all metrics are Rust (depecher-analyse)
- `/convert` pipeline: ContentVec → RMVPE (mel spectrogram) → Generator (chunked 200-frame windows)
- Session pool: ONNX sessions loaded once at `POST /models/:name`, reused across requests
- WAV cache: LRU(500) keyed by SHA-256(text + model + opts) in `AppState`
