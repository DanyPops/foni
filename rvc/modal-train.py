"""
RVC voice model training on Modal.

Usage:
    modal run rvc/modal-train.py --model sidorovich --epochs 500

Requires:
    pip install modal
    modal token new
"""

import modal
import os

app = modal.App("foni-rvc-train")

image = (
    modal.Image.from_registry(
        "docker.io/runpod/pytorch:2.2.0-py3.10-cuda12.1.1-devel-ubuntu22.04",
        add_python="3.10",
    )
    .apt_install("wget", "git", "ffmpeg")
    .run_commands(
        "git clone --depth=1 https://github.com/nakshatra-garg/rvc-no-gui.git /opt/rvc-no-gui",
        "cd /opt/rvc-no-gui && python pipeline.py setup",
    )
)

volume = modal.Volume.from_name("foni-training-data", create_if_missing=True)

DATASET_URL = "https://github.com/DanyPops/foni/releases/download/dataset-v1/foni-dataset.tar.gz"


@app.function(
    image=image,
    gpu="T4",
    timeout=3600,
    volumes={"/data": volume},
)
def train(model: str = "sidorovich", epochs: int = 500, batch_size: int = 8):
    import subprocess
    import glob
    import shutil

    dataset_dir = "/data/dataset"
    output_dir = "/data/output"

    os.makedirs(dataset_dir, exist_ok=True)
    os.makedirs(output_dir, exist_ok=True)

    # Download dataset if not cached in volume
    wavs = glob.glob(f"{dataset_dir}/*.wav")
    if not wavs:
        print(f"[train] downloading dataset from {DATASET_URL}")
        subprocess.run(
            f"curl -sL {DATASET_URL} | tar xzf - --no-same-owner -C {dataset_dir}",
            shell=True,
        )
        wavs = glob.glob(f"{dataset_dir}/*.wav")

    print(f"[train] {len(wavs)} WAV files, {epochs} epochs, batch={batch_size}")

    os.chdir("/opt/rvc-no-gui")
    wav_args = " ".join(f'"{w}"' for w in sorted(wavs))
    cmd = f"python pipeline.py train -m {model} -a {wav_args} -e {epochs} -b {batch_size}"
    print(f"[train] {cmd[:100]}...")
    subprocess.run(cmd, shell=True, check=True)

    # Copy model to output
    candidates = (
        glob.glob(f"weights/{model}.pth")
        + glob.glob(f"logs/{model}/*.pth")
    )
    if candidates:
        shutil.copy(candidates[0], f"{output_dir}/{model}.pth")
        size = os.path.getsize(f"{output_dir}/{model}.pth")
        print(f"[train] model saved: {output_dir}/{model}.pth ({size} bytes)")
    else:
        print("[train] WARNING: no model file found")

    volume.commit()
    print("[train] DONE")
    return f"{output_dir}/{model}.pth"


@app.local_entrypoint()
def main(model: str = "sidorovich", epochs: int = 500, batch_size: int = 8):
    result = train.remote(model=model, epochs=epochs, batch_size=batch_size)
    print(f"Model at: {result}")
