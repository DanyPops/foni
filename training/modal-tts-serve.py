"""
TTS inference server on Modal — Chatterbox + Fish Speech S2-Pro.

Deploys two endpoints for A/B comparison:
    POST /chatterbox  — Chatterbox Multilingual (500M, T4)
    POST /fish         — Fish Speech S2-Pro fine-tuned (4.6B, A100)

Usage:
    modal deploy training/modal-tts-serve.py
"""

import modal
import os
import io

app = modal.App("foni-tts-serve")

chatterbox_image = (
    modal.Image.debian_slim(python_version="3.11")
    .apt_install("ffmpeg", "libsndfile1")
    .pip_install("chatterbox-tts", "torchaudio", "scipy", "fastapi[standard]")
)

fish_image = (
    modal.Image.debian_slim(python_version="3.11")
    .apt_install("git", "ffmpeg", "libsndfile1")
    .run_commands(
        "git clone --depth=1 https://github.com/groxaxo/fish-speech-int4-patch.git /app",
        "cd /app && pip install -e '.[stable]' bitsandbytes 2>&1 | tail -5",
        'pip install huggingface_hub "fastapi[standard]" requests',
        "hf download groxaxo/s2-pro-BnB-4Bits --local-dir /app/checkpoints/s2-pro",
    )
)

volume = modal.Volume.from_name("foni-training", create_if_missing=True)

REF_WAV = "/data/dataset-raw/trader1a.wav"


@app.function(
    image=chatterbox_image,
    gpu="T4",
    volumes={"/data": volume},
    scaledown_window=300,
)
@modal.fastapi_endpoint(method="POST", label="chatterbox")
async def chatterbox(request: dict):
    """POST /chatterbox — Chatterbox Multilingual zero-shot."""
    import torch
    import torchaudio as ta
    from chatterbox.mtl_tts import ChatterboxMultilingualTTS
    from fastapi.responses import Response

    text = request.get("text", "")
    lang = request.get("language", "ru")
    if not text:
        return {"error": "no text"}

    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = ChatterboxMultilingualTTS.from_pretrained(device=device)

    wav = model.generate(text, audio_prompt_path=REF_WAV, language_id=lang)

    # Trim trailing breath artifacts
    sr = model.sr
    trim = int(0.3 * sr)
    if wav.shape[-1] > trim * 2:
        wav = wav[..., :-trim]

    buf = io.BytesIO()
    ta.save(buf, wav, sr, format="wav")
    return Response(content=buf.getvalue(), media_type="audio/wav")


@app.function(
    image=fish_image,
    gpu="T4",
    volumes={"/data": volume},
    scaledown_window=300,
)
@modal.fastapi_endpoint(method="POST", label="fish")
async def fish(request: dict):
    """POST /fish — Fish Speech S2-Pro fine-tuned on Sidorovich."""
    import subprocess
    import time
    import requests as http_requests
    from fastapi.responses import Response

    text = request.get("text", "")
    if not text:
        return {"error": "no text"}

    os.chdir("/app")

    # Symlink checkpoints
    if not os.path.exists("/app/checkpoints/s2-pro"):
        os.makedirs("/app/checkpoints", exist_ok=True)
        os.symlink("/data/checkpoints/s2-pro", "/app/checkpoints/s2-pro")

    # Start API server in background if not running
    try:
        http_requests.get("http://localhost:8080/v1/health", timeout=2)
    except Exception:
        print("[fish] starting API server...")
        subprocess.Popen(
            ["python", "tools/api_server.py",
             "--llama-checkpoint-path", "checkpoints/s2-pro",
             "--decoder-checkpoint-path", "checkpoints/s2-pro/codec.pth",
             "--bnb4", "--half",
             "--listen", "0.0.0.0:8080"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        # Wait for server to start
        for _ in range(60):
            try:
                http_requests.get("http://localhost:8080/v1/health", timeout=1)
                print("[fish] API server ready")
                break
            except Exception:
                time.sleep(2)

    # Call the API server
    resp = http_requests.post(
        "http://localhost:8080/v1/tts",
        files={
            "text": (None, text),
            "format": (None, "wav"),
            "reference_id": (None, "sidorovich"),
        },
        timeout=120,
    )

    if resp.status_code != 200:
        return {"error": f"API server returned {resp.status_code}: {resp.text[:200]}"}

    return Response(content=resp.content, media_type="audio/wav")


@app.function(image=chatterbox_image, volumes={"/data": volume})
@modal.fastapi_endpoint(method="GET", label="health")
async def health():
    return {"status": "ok", "models": ["chatterbox-multilingual", "fish-s2-pro-sidorovich"]}
