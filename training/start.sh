#!/usr/bin/env bash
# RVC API server with automatic model directory and default model setup.
# Runs the API server, waits for readiness, then configures it.

set -euo pipefail

MODELS_DIR="${RVC_MODELS_DIR:-/app/rvc_models}"
DEFAULT_MODEL="${RVC_DEFAULT_MODEL:-}"
PORT="${RVC_PORT:-5050}"

python -m rvc_python api -p "$PORT" -l &
SERVER_PID=$!

# Wait for server to be ready (max 30s)
for i in $(seq 1 30); do
    if curl -sf "http://localhost:$PORT/models" >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

curl -sf -X POST "http://localhost:$PORT/set_models_dir" \
    -H "Content-Type: application/json" \
    -d "{\"models_dir\": \"$MODELS_DIR\"}" >/dev/null

if [ -n "$DEFAULT_MODEL" ]; then
    curl -sf -X POST "http://localhost:$PORT/models/$DEFAULT_MODEL" >/dev/null \
        && echo "[rvc] model loaded: $DEFAULT_MODEL" \
        || echo "[rvc] model not found: $DEFAULT_MODEL (will load on first request)"
fi

echo "[rvc] ready on :$PORT (models: $MODELS_DIR)"
wait "$SERVER_PID"
