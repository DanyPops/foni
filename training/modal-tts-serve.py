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
    .pip_install("chatterbox-tts", "torchaudio", "scipy")
)

fish_image = (
    modal.Image.from_registry(
        "fishaudio/fish-speech:latest",
        setup_dockerfile_commands=[
            'ENTRYPOINT []',
            'ENV PATH="/app/.venv/bin:$PATH"',
        ],
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
    gpu="A10G",
    volumes={"/data": volume},
    scaledown_window=300,
)
@modal.fastapi_endpoint(method="POST", label="fish")
async def fish(request: dict):
    """POST /fish — Fish Speech S2-Pro fine-tuned on Sidorovich."""
    import subprocess
    import tempfile
    from fastapi.responses import Response

    text = request.get("text", "")
    if not text:
        return {"error": "no text"}

    os.chdir("/app")

    # Symlink fine-tuned model
    ckpt = "/app/checkpoints/sidorovich"
    if not os.path.exists(ckpt):
        os.makedirs("/app/checkpoints", exist_ok=True)
        if os.path.exists("/data/output/sidorovich"):
            os.symlink("/data/output/sidorovich", ckpt)
        elif os.path.exists("/data/checkpoints/s2-pro"):
            os.symlink("/data/checkpoints/s2-pro", ckpt)

    # Generate reference VQ tokens
    ref_npy = "/tmp/ref.npy"
    if not os.path.exists(ref_npy):
        subprocess.run([
            "/app/.venv/bin/python", "fish_speech/models/dac/inference.py",
            "-i", REF_WAV,
            "--checkpoint-path", f"{ckpt}/codec.pth",
            "--output-path", "/tmp/ref",
        ], check=True, capture_output=True)

    # Generate semantic tokens
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run([
            "/app/.venv/bin/python", "fish_speech/models/text2semantic/inference.py",
            "--text", text,
            "--prompt-tokens", ref_npy,
            "--checkpoint-path", ckpt,
            "--num-samples", "1",
        ], check=True, capture_output=True, cwd=tmp)

        codes = os.path.join(tmp, "codes_0.npy")
        if not os.path.exists(codes):
            return {"error": "no codes generated"}

        # Decode to audio
        subprocess.run([
            "/app/.venv/bin/python", "fish_speech/models/dac/inference.py",
            "-i", codes,
            "--checkpoint-path", f"{ckpt}/codec.pth",
            "--output-path", os.path.join(tmp, "output"),
        ], check=True, capture_output=True)

        wav_path = os.path.join(tmp, "output.wav")
        if not os.path.exists(wav_path):
            return {"error": "no audio generated"}

        with open(wav_path, "rb") as f:
            return Response(content=f.read(), media_type="audio/wav")


@app.function(image=chatterbox_image, volumes={"/data": volume})
@modal.fastapi_endpoint(method="GET", label="health")
async def health():
    return {"status": "ok", "models": ["chatterbox-multilingual", "fish-s2-pro-sidorovich"]}
