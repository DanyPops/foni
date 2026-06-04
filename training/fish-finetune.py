"""
Fish Speech S2 fine-tuning script for cloud GPU pods.

Dataset format: transcripts.txt with lines "filename.wav|Transcribed text"
Expects WAVs in FONI_DATASET_URL tar.gz alongside transcripts.txt.

Steps:
    1. Clone fish-speech, download model weights
    2. Prepare dataset (WAV → .lab files in speaker dir)
    3. Extract semantic tokens (GPU)
    4. Pack to protobuf
    5. LoRA fine-tune
    6. Merge LoRA weights
    7. Upload result

Env vars:
    FONI_MODEL        — speaker/model name (default: sidorovich)
    FONI_EPOCHS       — training steps (default: 100)
    FONI_DATASET_URL  — tar.gz with WAVs + transcripts.txt
    FONI_WORKSPACE    — working directory (default: /workspace)
    GITHUB_TOKEN      — for uploading result
    FONI_UPLOAD_TAG   — GitHub release tag
    FONI_REPO         — GitHub repo
"""

import glob
import json
import os
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path


@dataclass
class FinetuneConfig:
    model: str
    steps: int
    dataset_url: str
    workspace: Path
    upload_token: str
    upload_repo: str
    upload_tag: str

    @staticmethod
    def from_env() -> "FinetuneConfig":
        model = os.environ.get("FONI_MODEL", "sidorovich")
        return FinetuneConfig(
            model=model,
            steps=int(os.environ.get("FONI_EPOCHS", "100")),
            dataset_url=os.environ.get("FONI_DATASET_URL", ""),
            workspace=Path(os.environ.get("FONI_WORKSPACE", "/workspace")),
            upload_token=os.environ.get("GITHUB_TOKEN", ""),
            upload_repo=os.environ.get("FONI_REPO", "DanyPops/foni"),
            upload_tag=os.environ.get("FONI_UPLOAD_TAG", f"model-{model}"),
        )

    @property
    def fish_dir(self) -> Path:
        return self.workspace / "fish-speech"

    @property
    def data_dir(self) -> Path:
        return self.workspace / "data" / self.model

    @property
    def output_dir(self) -> Path:
        return self.workspace / "output"


def run(cmd: list[str] | str, **kwargs) -> subprocess.CompletedProcess:
    label = cmd if isinstance(cmd, str) else " ".join(cmd)
    print(f"  $ {label[:120]}", flush=True)
    return subprocess.run(cmd, check=True, **kwargs)


def install_fish_speech(cfg: FinetuneConfig):
    """Clone fish-speech and install dependencies."""
    if not cfg.fish_dir.exists():
        print("[setup] cloning fish-speech...", flush=True)
        run(["git", "clone", "--depth=1",
             "https://github.com/fishaudio/fish-speech.git",
             str(cfg.fish_dir)])

    os.chdir(cfg.fish_dir)
    print("[setup] installing fish-speech...", flush=True)
    run([sys.executable, "-m", "pip", "install", "-q", "-e", "."])

    print("[setup] downloading model weights...", flush=True)
    run(["huggingface-cli", "download", "fishaudio/openaudio-s1-mini",
         "--local-dir", "checkpoints/openaudio-s1-mini"])


def download_dataset(cfg: FinetuneConfig) -> int:
    """Download dataset and prepare speaker directory with .lab files."""
    raw_dir = cfg.workspace / "dataset-raw"
    raw_dir.mkdir(parents=True, exist_ok=True)
    cfg.data_dir.mkdir(parents=True, exist_ok=True)

    if cfg.dataset_url:
        print(f"[dataset] downloading from {cfg.dataset_url}", flush=True)
        run(f"curl -sL {cfg.dataset_url} | tar xzf - --no-same-owner -C {raw_dir}",
            shell=True)

    # Read transcripts
    transcripts_path = raw_dir / "transcripts.txt"
    if not transcripts_path.exists():
        raise FileNotFoundError(f"No transcripts.txt in {raw_dir}")

    transcripts = {}
    for line in transcripts_path.read_text(encoding="utf-8").splitlines():
        if "|" not in line:
            continue
        filename, text = line.split("|", 1)
        transcripts[filename.strip()] = text.strip()

    # Copy WAVs and create .lab files
    count = 0
    for wav in sorted(raw_dir.glob("*.wav")):
        text = transcripts.get(wav.name)
        if not text:
            print(f"  skip {wav.name}: no transcript", flush=True)
            continue
        shutil.copy(wav, cfg.data_dir / wav.name)
        lab_path = cfg.data_dir / f"{wav.stem}.lab"
        lab_path.write_text(text, encoding="utf-8")
        count += 1

    print(f"[dataset] {count} files with transcripts", flush=True)
    if count == 0:
        raise FileNotFoundError("No WAV files matched transcripts")
    return count


def extract_tokens(cfg: FinetuneConfig):
    """Extract semantic tokens from audio using VQGAN."""
    os.chdir(cfg.fish_dir)
    # Go up one level — data dir should be relative to fish-speech
    data_parent = cfg.data_dir.parent
    run([
        sys.executable, "tools/vqgan/extract_vq.py", str(data_parent),
        "--num-workers", "1", "--batch-size", "16",
        "--config-name", "modded_dac_vq",
        "--checkpoint-path", "checkpoints/openaudio-s1-mini/codec.pth",
    ])


def build_dataset(cfg: FinetuneConfig):
    """Pack dataset into protobuf format."""
    os.chdir(cfg.fish_dir)
    data_parent = cfg.data_dir.parent
    run([
        sys.executable, "tools/llama/build_dataset.py",
        "--input", str(data_parent),
        "--output", str(data_parent / "protos"),
        "--text-extension", ".lab",
        "--num-workers", "4",
    ])


def finetune(cfg: FinetuneConfig):
    """Run LoRA fine-tuning."""
    os.chdir(cfg.fish_dir)
    run([
        sys.executable, "fish_speech/train.py",
        "--config-name", "text2semantic_finetune",
        f"project={cfg.model}",
        "+lora@model.model.lora_config=r_8_alpha_16",
        f"trainer.max_steps={cfg.steps}",
    ])


def merge_lora(cfg: FinetuneConfig) -> Path:
    """Merge LoRA weights into base model."""
    os.chdir(cfg.fish_dir)

    # Find the latest checkpoint
    ckpt_dir = cfg.fish_dir / "results" / cfg.model / "checkpoints"
    checkpoints = sorted(ckpt_dir.glob("*.ckpt"))
    if not checkpoints:
        raise FileNotFoundError(f"No checkpoints in {ckpt_dir}")

    latest = checkpoints[-1]
    output = cfg.fish_dir / f"checkpoints/{cfg.model}-finetuned"

    print(f"[merge] {latest.name} → {output}", flush=True)
    run([
        sys.executable, "tools/llama/merge_lora.py",
        "--lora-config", "r_8_alpha_16",
        "--base-weight", "checkpoints/openaudio-s1-mini",
        "--lora-weight", str(latest),
        "--output", str(output),
    ])
    return output


def upload_model(cfg: FinetuneConfig, model_dir: Path, elapsed: float):
    """Upload fine-tuned model to GitHub release."""
    if not cfg.upload_token:
        print("[upload] GITHUB_TOKEN not set, skipping", flush=True)
        return

    # Tar the model directory
    tar_path = cfg.output_dir / f"{cfg.model}-fish.tar.gz"
    cfg.output_dir.mkdir(parents=True, exist_ok=True)
    run(f"tar czf {tar_path} -C {model_dir.parent} {model_dir.name}", shell=True)

    api = f"https://api.github.com/repos/{cfg.upload_repo}"
    auth = f"Authorization: token {cfg.upload_token}"
    tag = cfg.upload_tag
    body = json.dumps({
        "tag_name": tag,
        "name": f"{cfg.model} Fish Speech model",
        "body": f"steps={cfg.steps} time={elapsed:.0f}s",
    })

    subprocess.run(
        ["curl", "-sX", "DELETE", "-H", auth, f"{api}/releases/tags/{tag}"],
        capture_output=True)

    result = subprocess.run(
        ["curl", "-s", "-H", auth, "-H", "Content-Type: application/json",
         f"{api}/releases", "-d", body],
        capture_output=True, text=True)
    release = json.loads(result.stdout)
    upload_url = release.get("upload_url", "").replace("{?name,label}", "")

    if upload_url:
        subprocess.run(
            ["curl", "-s", "-H", auth, "-H", "Content-Type: application/gzip",
             f"{upload_url}?name={cfg.model}-fish.tar.gz",
             "--data-binary", f"@{tar_path}"],
            capture_output=True)
        print(f"[upload] → {cfg.upload_repo}/releases/{tag}", flush=True)
    else:
        print(f"[upload] failed: {result.stdout[:200]}", flush=True)


def mark_complete(cfg: FinetuneConfig):
    cfg.output_dir.mkdir(parents=True, exist_ok=True)
    (cfg.output_dir / "COMPLETE").write_text("ok")


def mark_failed(cfg: FinetuneConfig, reason: str):
    cfg.output_dir.mkdir(parents=True, exist_ok=True)
    (cfg.output_dir / "FAILED").write_text(reason)


def main():
    cfg = FinetuneConfig.from_env()
    print(f"[main] model={cfg.model} steps={cfg.steps}", flush=True)

    try:
        install_fish_speech(cfg)
        download_dataset(cfg)

        t0 = time.time()
        extract_tokens(cfg)
        build_dataset(cfg)
        finetune(cfg)
        model_dir = merge_lora(cfg)
        elapsed = time.time() - t0

        print(f"[main] fine-tuning completed in {elapsed:.0f}s", flush=True)
        upload_model(cfg, model_dir, elapsed)
        mark_complete(cfg)
        print("[main] DONE", flush=True)

    except Exception as e:
        print(f"[main] FAILED: {e}", flush=True)
        import traceback
        traceback.print_exc()
        mark_failed(cfg, str(e))
        sys.exit(1)


if __name__ == "__main__":
    main()
