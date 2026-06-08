# Foni — task runner
# https://github.com/casey/just

server_url  := env("FONI_SYNTH_URL",  "http://localhost:5050")
models_dir  := env("RVC_MODELS_DIR",  "rvc/models")
model       := env("FONI_MODEL",       "sidorovich")
pool_size   := env("FONI_POOL_SIZE",   "4")
fonictl     := "foni-server/target/debug/fonictl"
reference   := "baseline/stalker/wav/sidorovich/trader1a.wav"
presets     := "foni-maquettes.json"

# ── Quality gate ───────────────────────────────────────────────────────────────

# Full quality gate: typecheck → tests → clippy → cargo test
[group('ci')]
check: ts-check test lint rust-test

# TypeScript typecheck
[group('ci')]
ts-check:
    npx tsc --noEmit

# Run all unit tests (no E2E, no live services needed)
[group('ci')]
test:
    npx vitest run --exclude '**/*.e2e*'

# Watch mode
[group('ci')]
test-watch:
    npx vitest

# Rust lint + format check
[group('ci')]
lint:
    cd foni-server && cargo fmt --all -- --check
    cd foni-server && cargo clippy --workspace --all-targets --quiet

# Rust unit tests
[group('ci')]
rust-test:
    cd foni-server && cargo test --workspace --lib --quiet

# Pool concurrency tests (requires server up + model loaded)
[group('ci')]
rust-test-pool:
    cd foni-server && FONI_TEST_URL={{server_url}} \
        cargo test -p foni-synth --test pool_concurrency -- --nocapture

# ── Build ──────────────────────────────────────────────────────────────────────

# Build debug binaries
[group('build')]
build:
    cd foni-server && cargo build --workspace

# Build release binaries (used by systemd service)
[group('build')]
build-release:
    cd foni-server && cargo build --workspace --release

# Format all Rust code
[group('build')]
fmt:
    cd foni-server && cargo fmt --all

# ── Server management ──────────────────────────────────────────────────────────

# Start foni-synth via systemd (release binary, auto-restart)
[group('server')]
start:
    systemctl --user start foni-synth
    @echo "Waiting for server..."
    @sleep 5
    @curl -sf {{server_url}}/params > /dev/null && echo "✓ up" || echo "✗ not responding"

# Stop foni-synth
[group('server')]
stop:
    systemctl --user stop foni-synth

# Restart and pre-warm model
[group('server')]
restart: stop start warm

# Server status + pool metrics
[group('server')]
status:
    systemctl --user status foni-synth --no-pager | head -10
    @echo ""
    @curl -s {{server_url}}/metrics | python3 -m json.tool

# Pre-warm model sessions (fills all 4 pool slots)
[group('server')]
warm:
    curl -sf -X POST {{server_url}}/models/{{model}} \
        && echo "✓ {{model}} loaded" \
        || echo "✗ warm failed — is the server up?"

# Server logs (live)
[group('server')]
logs:
    journalctl --user -u foni-synth -f

# ── Audio tuning ───────────────────────────────────────────────────────────────

# ★ Interactive preset tuner — plays reference then synthetic, rate each 1-5
[group('tune')]
tune:
    FONI_SYNTH_URL={{server_url}} {{fonictl}} tune \
        --presets {{presets}} \
        --reference {{reference}} \
        -m {{model}}

# Auto-tune N iterations — coordinate descent over DSP knobs, saves top-3 to foni-maquettes.json
# Then run `just tune` to listen to the results
[group('tune')]
auto iterations="20":
    FONI_SYNTH_URL={{server_url}} {{fonictl}} tune \
        --auto {{iterations}} \
        --presets {{presets}} \
        --vs {{reference}} \
        -m {{model}}

# Interactive ratatui mixer TUI (needs a real terminal)
[group('tune')]
mix:
    FONI_SYNTH_URL={{server_url}} {{fonictl}} mix \
        --from {{presets}} \
        --reference {{reference}} \
        -m {{model}}

# DSP isolation — play each stage in order, find buzz/wobble source
[group('tune')]
diagnose:
    FONI_SYNTH_URL={{server_url}} {{fonictl}} listen --diagnose -m {{model}}

# ── Acoustic analysis ──────────────────────────────────────────────────────────

# Sidorovich corpus fingerprint (63 studio recordings)
[group('analyse')]
fingerprint:
    FONI_SYNTH_URL={{server_url}} \
        {{fonictl}} corpus baseline/stalker/wav/sidorovich/ \
        --vs {{reference}}

# 1:1 studio vs synthetic gap report (transcribes + synthesizes matched pairs)
[group('analyse')]
compare:
    FONI_SYNTH_URL={{server_url}} {{fonictl}} compare \
        baseline/stalker/wav/sidorovich/ \
        --max-dur 5.0 \
        --out-dir /tmp/fonictl_compare \
        --model {{model}}

# Analyse a single WAV vs studio reference
[group('analyse')]
analyse wav:
    {{fonictl}} analyse {{wav}} --vs {{reference}}

# ── Tier B scoring (UTMOSv2 MOS + ECAPA speaker similarity) ──────────────────────

# Check a WAV against studio reference — naturalness score + voice match
[group('analyse')]
check-wav wav:
    FONI_SYNTH_URL={{server_url}} {{fonictl}} score {{wav}} \
        --reference {{reference}} \
        --voice-id rvc/models/pretrained/ecapa-voxceleb.onnx

# Learn Sidorovich — build voice fingerprint from all studio recordings (run once)
[group('analyse')]
learn-sidorovich:
    FONI_SYNTH_URL={{server_url}} {{fonictl}} score \
        --dir baseline/stalker/wav/sidorovich/ \
        --reference {{reference}} \
        --voice-id rvc/models/pretrained/ecapa-voxceleb.onnx \
        --save-mean baseline/stalker/scores/sidorovich_voice_mean.npy

# Full voice report — synthesize a phrase and show all quality numbers
[group('analyse')]
voice-report:
    FONI_SYNTH_URL={{server_url}} {{fonictl}} score --synthesize \
        --text "Здравствуй, сталкер. Чего тебе надо?" \
        --model {{model}} \
        --reference {{reference}} \
        --voice-id rvc/models/pretrained/ecapa-voxceleb.onnx

# ── Model setup (one-time) ─────────────────────────────────────────────────────

# Set up the voice identity model — needed once before running `just check` or `just voice-report`
[group('setup')]
setup-voice-id:
    podman run --rm \
        -v {{justfile_directory()}}:/foni:Z \
        -w /foni \
        localhost/foni-rvc \
        bash -c 'pip install -q speechbrain && python3 rvc/export_ecapa_onnx.py'

# Export ONNX generator for a model using the foni-rvc container (torch already installed)
# Usage: just export-model sidorovich   or   just export-model bandit
[group('setup')]
export-model model:
    #!/usr/bin/env bash
    set -euo pipefail
    RVC_SRC=/tmp/rvc-onnx-source
    if [ ! -d "$RVC_SRC" ]; then
        echo "Cloning RVC source..."
        git clone --depth=1 --filter=blob:none --sparse \
            https://github.com/RVC-Project/Retrieval-based-Voice-Conversion-WebUI \
            "$RVC_SRC"
        cd "$RVC_SRC" && git sparse-checkout set infer/lib/infer_pack
    fi
    podman run --rm \
        -v {{justfile_directory()}}:/foni:Z \
        -v "$RVC_SRC:$RVC_SRC:Z" \
        -w /foni \
        localhost/foni-rvc \
        bash -c 'pip install -q onnx==1.15.0 onnxruntime==1.17.3 && python3 rvc/export_onnx.py {{model}}'

# Export FAISS voice index vectors to .npy using foni-rvc container
[group('setup')]
export-index:
    podman run --rm \
        -v {{justfile_directory()}}:/foni:Z \
        -w /foni \
        localhost/foni-rvc \
        python3 rvc/export_voice_index.py

# ── Git ────────────────────────────────────────────────────────────────────────

# Stage all, commit, push to danypops
[group('git')]
ship msg:
    git add -A
    git commit -m "{{msg}}"
    git push danypops

# Push current branch
[group('git')]
push:
    git push danypops

# ── Shortcuts ──────────────────────────────────────────────────────────────────

# Default: show available recipes
[private]
default:
    @just --list

# Security gate — run on every commit (Swiss Cheese layer 1: detect before git)
security-check:
    @echo "=== Secret scan ==="
    python3.11 -m detect_secrets scan \
        --exclude-files '\.git|node_modules|target|training/stress|\.wav$|\.mp4$|\.onnx$|\.baseline$' \
        --baseline .secrets.baseline
    @echo "=== No hardcoded text in logs ==="
    @! grep -rn 'tracing::info!.*text\s*=\s*%' \
        foni-server/foni-synth/src/ foni-server/foni-cli/src/ 2>/dev/null \
        | grep -v 'chars\|len\|count\|//\|#\[' \
        | grep . && echo "✅ no raw text in tracing spans" || (echo "❌ raw text found in logs"; exit 1)
    @echo "=== Token not in service files ==="
    @! grep -r 'FONI_TTS_TOKEN\s*=\s*[A-Za-z0-9+/]\{20,\}' \
        ~/.config/systemd/ 2>/dev/null \
        | grep . && echo "✅ no hardcoded token in systemd units" || (echo "❌ token found in service file"; exit 1)
    @echo "=== Rust contract tests ==="
    cd foni-server && cargo test --test tts_contract 2>&1 | tail -3
    @echo "✅ security-check passed"
