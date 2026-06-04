"""
Test CosyVoice 2 zero-shot voice cloning with Sidorovich.

Usage:
    modal run training/modal-cosyvoice-test.py
"""

import modal
import os

app = modal.App("foni-cosyvoice-test")

image = (
    modal.Image.debian_slim(python_version="3.10")
    .apt_install("git", "ffmpeg", "libsndfile1", "sox")
    .pip_install("torch==2.6.0", "torchaudio==2.6.0")
    .run_commands(
        "git clone --depth=1 --recursive https://github.com/FunAudioLLM/CosyVoice.git /opt/cosyvoice",
        "cd /opt/cosyvoice && pip install -r requirements.txt 2>&1 | tail -3",
        "cd /opt/cosyvoice/third_party/Matcha-TTS && pip install -e . 2>&1 | tail -3",
        "pip install modelscope hyperpyyaml onnxruntime openai-whisper conformer inflect pydantic",
    )
)

volume = modal.Volume.from_name("foni-training", create_if_missing=True)


@app.function(
    image=image,
    gpu="T4",
    timeout=600,
    volumes={"/data": volume},
)
def test_cosyvoice():
    import sys
    import time

    sys.path.insert(0, "/opt/cosyvoice")
    sys.path.insert(0, "/opt/cosyvoice/third_party/Matcha-TTS")
    os.chdir("/opt/cosyvoice")

    from modelscope import snapshot_download
    model_dir = snapshot_download("iic/CosyVoice2-0.5B", cache_dir="/data/cosyvoice-models")
    print(f"[test] model: {model_dir}")

    from cosyvoice.cli.cosyvoice import CosyVoice2
    import torchaudio

    t0 = time.time()
    model = CosyVoice2(model_dir, load_jit=False, load_trt=False)
    print(f"[test] loaded in {time.time() - t0:.1f}s")

    ref_wav = "/data/dataset-raw/trader1a.wav"
    ref_text = "Подойди-ка, надо тебе ситуацию прояснить."

    phrases = [
        "Привет, сталкер. Чего тебе надо?",
        "Деплой прошёл успешно, коммиты запушены.",
        "Удачи, браток. На Зоне удача нужна.",
    ]

    os.makedirs("/data/cosyvoice-test", exist_ok=True)

    for i, text in enumerate(phrases):
        t0 = time.time()
        print(f"[test] [{i+1}/{len(phrases)}]: {text}")
        for result in model.inference_zero_shot(text, ref_text, ref_wav, stream=False):
            wav = result["tts_speech"]
            out = f"/data/cosyvoice-test/sidorovich_{i:02d}.wav"
            torchaudio.save(out, wav, 22050)
            elapsed = time.time() - t0
            dur = wav.shape[-1] / 22050
            print(f"[test]   {dur:.1f}s audio in {elapsed:.1f}s (RTF={elapsed/dur:.1f}x)")

    volume.commit()
    print("[test] DONE")
    return "/data/cosyvoice-test/"


@app.local_entrypoint()
def main():
    result = test_cosyvoice.remote()
    print(f"Result: {result}")
