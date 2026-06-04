"""
Fish Speech S2 fine-tuning on Modal.

Usage:
    modal run training/modal-train.py --model sidorovich --steps 10      # smoke test
    modal run training/modal-train.py --model sidorovich --steps 500     # real training
"""

import modal
import os

app = modal.App("foni-fish-finetune")

image = (
    modal.Image.from_registry("fishaudio/fish-speech:latest", add_python="3.12")
    .apt_install("curl")
    .pip_install("huggingface_hub")
    .run_commands(
        # s2-pro is public, no approval needed
        "hf download fishaudio/s2-pro --local-dir /opt/fish-speech/checkpoints/s2-pro",
    )
)

CHECKPOINT = "checkpoints/s2-pro"

volume = modal.Volume.from_name("foni-training", create_if_missing=True)


@app.function(
    image=image,
    gpu="L4",
    timeout=7200,
    volumes={"/data": volume},
)
def train(model: str = "sidorovich", steps: int = 100):
    """Fine-tune Fish Speech S2 with LoRA on Sidorovich dataset."""
    import glob
    import shutil
    import subprocess
    import time

    raw_dir = "/data/dataset-raw"
    data_dir = f"/data/{model}"
    os.makedirs(data_dir, exist_ok=True)

    wavs = sorted(glob.glob(f"{raw_dir}/*.wav"))
    if not wavs:
        raise RuntimeError(f"No WAV files in {raw_dir} — upload dataset to volume first")

    print(f"[train] {len(wavs)} WAV files in {raw_dir}")

    # Read transcripts → .lab files
    transcripts = {}
    tx_path = f"{raw_dir}/transcripts.txt"
    if os.path.exists(tx_path):
        for line in open(tx_path, encoding="utf-8"):
            if "|" in line:
                fname, text = line.strip().split("|", 1)
                transcripts[fname] = text

    count = 0
    for wav in wavs:
        name = os.path.basename(wav)
        text = transcripts.get(name)
        if not text:
            continue
        shutil.copy(wav, f"{data_dir}/{name}")
        stem = os.path.splitext(name)[0]
        with open(f"{data_dir}/{stem}.lab", "w", encoding="utf-8") as f:
            f.write(text)
        count += 1

    print(f"[train] {count} files with transcripts ready")

    os.chdir("/opt/fish-speech")

    print("[train] extracting semantic tokens...")
    subprocess.run([
        "python", "tools/vqgan/extract_vq.py", "/data",
        "--num-workers", "1", "--batch-size", "16",
        "--config-name", "modded_dac_vq",
        "--checkpoint-path", "checkpoints/s2-pro/codec.pth",
    ], check=True)

    print("[train] building protobuf dataset...")
    subprocess.run([
        "python", "tools/llama/build_dataset.py",
        "--input", "/data",
        "--output", "/data/protos",
        "--text-extension", ".lab",
        "--num-workers", "4",
    ], check=True)

    print(f"[train] fine-tuning {steps} steps...")
    t0 = time.time()
    subprocess.run([
        "python", "fish_speech/train.py",
        "--config-name", "text2semantic_finetune",
        f"project={model}",
        "+lora@model.model.lora_config=r_8_alpha_16",
        f"trainer.max_steps={steps}",
    ], check=True)
    elapsed = time.time() - t0
    print(f"[train] training done in {elapsed:.0f}s")

    # Merge LoRA
    ckpt_dir = f"/opt/fish-speech/results/{model}/checkpoints"
    checkpoints = sorted(glob.glob(f"{ckpt_dir}/*.ckpt"))
    if checkpoints:
        output_dir = f"/data/output/{model}"
        os.makedirs(output_dir, exist_ok=True)
        print(f"[train] merging LoRA: {checkpoints[-1]}")
        subprocess.run([
            "python", "tools/llama/merge_lora.py",
            "--lora-config", "r_8_alpha_16",
            "--base-weight", "checkpoints/s2-pro",
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
def main(model: str = "sidorovich", steps: int = 100):
    print(f"Fish Speech fine-tuning: model={model}, steps={steps}")
    result = train.remote(model=model, steps=steps)
    print(f"Result: {result}")
