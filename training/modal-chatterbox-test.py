"""
Test Chatterbox Multilingual zero-shot voice cloning with Sidorovich.

Usage:
    modal run training/modal-chatterbox-test.py
"""

import modal
import os

app = modal.App("foni-chatterbox-test")

image = (
    modal.Image.debian_slim(python_version="3.11")
    .apt_install("ffmpeg", "libsndfile1")
    .pip_install("chatterbox-tts", "torchaudio", "scipy")
)

volume = modal.Volume.from_name("foni-training", create_if_missing=True)


@app.function(
    image=image,
    gpu="T4",
    timeout=600,
    volumes={"/data": volume},
)
def test_chatterbox():
    """Test zero-shot voice cloning with Sidorovich WAV."""
    import torch
    import torchaudio as ta
    import glob

    device = "cuda" if torch.cuda.is_available() else "cpu"
    print(f"[test] device: {device}")

    # Find a good reference WAV
    wavs = sorted(glob.glob("/data/dataset-raw/*.wav"))
    if not wavs:
        raise RuntimeError("No WAVs in /data/dataset-raw/")

    # Pick trader1a — the iconic Sidorovich line
    ref_wav = next((w for w in wavs if "trader1a" in w), wavs[0])
    print(f"[test] reference: {os.path.basename(ref_wav)}")

    # Load Chatterbox Multilingual
    print("[test] loading Chatterbox Multilingual...")
    from chatterbox.mtl_tts import ChatterboxMultilingualTTS
    model = ChatterboxMultilingualTTS.from_pretrained(device=device)
    print("[test] model loaded")

    # Generate Russian speech in Sidorovich's voice
    phrases = [
        "Привет, сталкер. Чего тебе надо?",
        "Осторожно. Здесь аномалии, не зевай.",
        "Деплой прошёл успешно, коммиты запушены.",
        "Удачи, браток. На Зоне удача нужна.",
    ]

    os.makedirs("/data/chatterbox-test", exist_ok=True)

    for i, text in enumerate(phrases):
        print(f"[test] generating [{i+1}/{len(phrases)}]: {text}")
        wav = model.generate(
            text,
            audio_prompt_path=ref_wav,
            language_id="ru",
        )
        out_path = f"/data/chatterbox-test/sidorovich_{i:02d}.wav"
        ta.save(out_path, wav, model.sr)
        print(f"[test] saved: {out_path} ({wav.shape[-1] / model.sr:.1f}s)")

    volume.commit()
    print("[test] DONE — files saved to volume at /data/chatterbox-test/")
    return "/data/chatterbox-test/"


@app.local_entrypoint()
def main():
    result = test_chatterbox.remote()
    print(f"Result: {result}")
