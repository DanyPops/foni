"""
Fish Speech S2 fine-tuning on Modal.

Usage:
    modal run rvc/modal-train.py --model sidorovich --epochs 100

Prerequisites:
    pip install modal
    modal token new
"""

import modal
import os

app = modal.App("foni-fish-finetune")

# Image with fish-speech and all deps pre-installed
image = (
    modal.Image.debian_slim(python_version="3.12")
    .apt_install("git", "ffmpeg", "libsndfile1", "wget")
    .pip_install("torch", "torchaudio", index_url="https://download.pytorch.org/whl/cu126")
    .run_commands(
        "git clone --depth=1 https://github.com/fishaudio/fish-speech.git /opt/fish-speech",
        "cd /opt/fish-speech && pip install -e .",
        "huggingface-cli download fishaudio/openaudio-s1-mini --local-dir /opt/fish-speech/checkpoints/openaudio-s1-mini",
    )
)

# Persistent volume for dataset + model output
volume = modal.Volume.from_name("foni-training", create_if_missing=True)

DATASET_URL = "https://github.com/DanyPops/foni/releases/download/dataset-fish/foni-dataset-fish.tar.gz"


@app.function(
    image=image,
    gpu="A100",
    timeout=3600,
    volumes={"/data": volume},
)
def train(model: str = "sidorovich", epochs: int = 100):
    """Fine-tune Fish Speech S2 with LoRA on Sidorovich dataset."""
    import glob
    import shutil
    import subprocess
    import time

    data_dir = f"/data/{model}"
    raw_dir = "/data/dataset-raw"

    os.makedirs(data_dir, exist_ok=True)
    os.makedirs(raw_dir, exist_ok=True)

    # Download dataset if not cached in volume
    if not glob.glob(f"{raw_dir}/*.wav"):
        print(f"[train] downloading dataset...")
        subprocess.run(
            f"curl -sL {DATASET_URL} | tar xzf - --no-same-owner -C {raw_dir}",
            shell=True,
        )

    # Read transcripts and create .lab files
    transcripts = {}
    transcripts_path = f"{raw_dir}/transcripts.txt"
    if os.path.exists(transcripts_path):
        for line in open(transcripts_path, encoding="utf-8"):
            if "|" in line:
                fname, text = line.strip().split("|", 1)
                transcripts[fname] = text

    count = 0
    for wav in sorted(glob.glob(f"{raw_dir}/*.wav")):
        name = os.path.basename(wav)
        text = transcripts.get(name)
        if not text:
            continue
        shutil.copy(wav, f"{data_dir}/{name}")
        stem = os.path.splitext(name)[0]
        with open(f"{data_dir}/{stem}.lab", "w", encoding="utf-8") as f:
            f.write(text)
        count += 1

    print(f"[train] {count} files with transcripts")

    # Run Fish Speech fine-tuning pipeline
    os.chdir("/opt/fish-speech")

    print("[train] extracting semantic tokens...")
    subprocess.run([
        "python", "tools/vqgan/extract_vq.py", "/data",
        "--num-workers", "1", "--batch-size", "16",
        "--config-name", "modded_dac_vq",
        "--checkpoint-path", "checkpoints/openaudio-s1-mini/codec.pth",
    ], check=True)

    print("[train] building dataset...")
    subprocess.run([
        "python", "tools/llama/build_dataset.py",
        "--input", "/data",
        "--output", "/data/protos",
        "--text-extension", ".lab",
        "--num-workers", "4",
    ], check=True)

    print(f"[train] fine-tuning {epochs} steps...")
    t0 = time.time()
    subprocess.run([
        "python", "fish_speech/train.py",
        "--config-name", "text2semantic_finetune",
        f"project={model}",
        "+lora@model.model.lora_config=r_8_alpha_16",
        f"trainer.max_steps={epochs}",
    ], check=True)
    elapsed = time.time() - t0
    print(f"[train] training done in {elapsed:.0f}s")

    # Merge LoRA
    ckpt_dir = f"/opt/fish-speech/results/{model}/checkpoints"
    checkpoints = sorted(glob.glob(f"{ckpt_dir}/*.ckpt"))
    if checkpoints:
        output_dir = f"/data/output/{model}"
        print(f"[train] merging LoRA: {checkpoints[-1]}")
        subprocess.run([
            "python", "tools/llama/merge_lora.py",
            "--lora-config", "r_8_alpha_16",
            "--base-weight", "checkpoints/openaudio-s1-mini",
            "--lora-weight", checkpoints[-1],
            "--output", output_dir,
        ], check=True)
        print(f"[train] model saved to {output_dir}")
    else:
        print("[train] WARNING: no checkpoints found")

    volume.commit()
    print("[train] DONE")
    return f"/data/output/{model}"


@app.local_entrypoint()
def main(model: str = "sidorovich", epochs: int = 100):
    print(f"Starting Fish Speech fine-tuning: {model}, {epochs} steps")
    result = train.remote(model=model, epochs=epochs)
    print(f"Model at: {result}")
