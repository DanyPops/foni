"""
TTS inference server on Modal — Chatterbox Multilingual.

Model and speaker embedding loaded once on container start.
Subsequent requests only run generation (~5-8s instead of ~19s).

Usage:
    modal deploy training/modal-tts-serve.py
    export FISH_SPEECH_URL=https://dpopsuev--foni-tts-serve-chatterbox-tts.modal.run
"""

import modal
import os
import io

app = modal.App("foni-tts-serve")

image = (
    modal.Image.debian_slim(python_version="3.11")
    .apt_install("ffmpeg", "libsndfile1")
    .pip_install("chatterbox-tts", "torchaudio", "scipy", "fastapi[standard]")
)

volume = modal.Volume.from_name("foni-training", create_if_missing=True)
tts_secret = modal.Secret.from_name("foni-tts-auth")

REF_WAV = "/data/dataset-raw/trader1a.wav"


@app.cls(
    image=image,
    gpu="T4",
    volumes={"/data": volume},
    max_containers=5,
    buffer_containers=1,
    scaledown_window=300,
    secrets=[tts_secret],
)
class ChatterboxTTS:
    @modal.enter()
    def load(self):
        import torch
        from chatterbox.mtl_tts import ChatterboxMultilingualTTS

        self.device = "cuda" if torch.cuda.is_available() else "cpu"
        print(f"[tts] loading model on {self.device}...")
        self.model = ChatterboxMultilingualTTS.from_pretrained(device=self.device)
        print("[tts] model loaded")

        print(f"[tts] pre-encoding voice from {REF_WAV}...")
        self.model.generate("тест", audio_prompt_path=REF_WAV, language_id="ru")
        print("[tts] voice cached — ready for requests")

    @modal.fastapi_endpoint(method="POST")
    async def tts(self, request: dict):
        from fastapi.responses import JSONResponse, Response
        import torchaudio as ta

        expected = os.environ.get("TTS_AUTH_TOKEN", "")
        token = request.get("token", "")
        if expected and token != expected:
            return JSONResponse({"error": "unauthorized"}, status_code=401)

        text = request.get("text", "")
        lang = request.get("language", "ru")
        if not text:
            return JSONResponse({"error": "no text"})

        exaggeration = float(request.get("exaggeration", 0.5))
        cfg_weight = float(request.get("cfg_weight", 0.5))
        temperature = float(request.get("temperature", 0.8))

        wav = self.model.generate(
            text,
            audio_prompt_path=REF_WAV,
            language_id=lang,
            exaggeration=exaggeration,
            cfg_weight=cfg_weight,
            temperature=temperature,
        )

        sr = self.model.sr
        trim = int(0.3 * sr)
        if wav.shape[-1] > trim * 2:
            wav = wav[..., :-trim]

        buf = io.BytesIO()
        ta.save(buf, wav, sr, format="wav")
        return Response(content=buf.getvalue(), media_type="audio/wav")

    @modal.fastapi_endpoint(method="GET")
    async def health(self):
        return {"status": "ok", "model": "chatterbox-multilingual", "device": self.device}


