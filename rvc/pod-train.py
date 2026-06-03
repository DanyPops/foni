"""
Headless RVC training script for cloud GPU pods.

Expects environment variables:
    FONI_MODEL        — model name (default: sidorovich)
    FONI_EPOCHS       — training epochs (default: 500)
    FONI_BATCH_SIZE   — batch size, scale with VRAM (default: 16)
    FONI_DATASET_URL  — tar.gz URL of WAV dataset
    GITHUB_TOKEN      — for uploading trained model to GitHub release
    FONI_UPLOAD_TAG   — GitHub release tag (default: model-{FONI_MODEL})
    FONI_REPO         — GitHub repo (default: DanyPops/foni)
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
class TrainConfig:
    model: str
    epochs: int
    batch_size: int
    dataset_url: str
    upload_token: str
    upload_repo: str
    upload_tag: str

    @staticmethod
    def from_env() -> "TrainConfig":
        model = os.environ.get("FONI_MODEL", "sidorovich")
        return TrainConfig(
            model=model,
            epochs=int(os.environ.get("FONI_EPOCHS", "500")),
            batch_size=int(os.environ.get("FONI_BATCH_SIZE", "16")),
            dataset_url=os.environ.get("FONI_DATASET_URL", ""),
            upload_token=os.environ.get("GITHUB_TOKEN", ""),
            upload_repo=os.environ.get("FONI_REPO", "DanyPops/foni"),
            upload_tag=os.environ.get("FONI_UPLOAD_TAG", f"model-{model}"),
        )


WORKSPACE = Path(os.environ.get("FONI_WORKSPACE", "/workspace"))
DATASET_DIR = WORKSPACE / "dataset"
OUTPUT_DIR = WORKSPACE / "output"
RVC_DIR = WORKSPACE / "rvc-no-gui"
RVC_REPO = "https://github.com/nakshatra-garg/rvc-no-gui.git"


def run(cmd: list[str] | str, **kwargs) -> subprocess.CompletedProcess:
    """Run a command, print it, and check for errors."""
    label = cmd if isinstance(cmd, str) else " ".join(cmd)
    print(f"  $ {label}")
    return subprocess.run(cmd, check=True, **kwargs)


CONDA_ENV = WORKSPACE / "rvc-env"
PYTHON = str(CONDA_ENV / "bin" / "python")


def setup_python():
    """Find a Python <=3.10 for RVC. Returns path to interpreter."""
    version = sys.version_info
    if version.minor <= 10:
        print(f"[setup] system Python {version.major}.{version.minor} is compatible")
        return sys.executable

    if Path(PYTHON).exists():
        print("[setup] conda Python 3.10 already installed")
        return PYTHON

    # Try conda if available
    import shutil
    if shutil.which("conda"):
        print(f"[setup] system Python {version.major}.{version.minor} too new, installing 3.10 via conda...")
        run(["conda", "create", "-y", "-p", str(CONDA_ENV), "python=3.10", "-q"])
        return PYTHON

    # No conda — warn and try system Python anyway
    print(f"[setup] WARNING: Python {version.major}.{version.minor} may be too new for RVC, no conda available")
    return sys.executable


def install_rvc():
    """Clone rvc-no-gui and install its dependencies."""
    python = setup_python()

    if not RVC_DIR.exists():
        print("[setup] cloning rvc-no-gui...")
        run(["git", "clone", "--depth=1", RVC_REPO, str(RVC_DIR)])

    os.chdir(RVC_DIR)
    run([python, "-m", "pip", "install", "-q", "-r", "requirements.txt"])
    run([python, "pipeline.py", "setup"])


def download_dataset(url: str) -> list[Path]:
    """Download and extract dataset, return list of WAV files."""
    DATASET_DIR.mkdir(parents=True, exist_ok=True)

    if url:
        print(f"[dataset] downloading from {url}")
        run(f"curl -sL {url} | tar xzf - --no-same-owner -C {DATASET_DIR}", shell=True)

    wavs = sorted(Path(f) for f in glob.glob(str(DATASET_DIR / "*.wav")))
    print(f"[dataset] {len(wavs)} WAV files")

    if not wavs:
        raise FileNotFoundError(f"no WAV files in {DATASET_DIR}")

    return wavs


def train(cfg: TrainConfig, wavs: list[Path]) -> Path:
    """Run RVC training, return path to trained model."""
    os.chdir(RVC_DIR)
    python = PYTHON if Path(PYTHON).exists() else sys.executable

    # subprocess list bypasses shell arg length limit
    cmd = [
        python, "pipeline.py", "train",
        "-m", cfg.model,
        "-a", *[str(w) for w in wavs],
        "-e", str(cfg.epochs),
        "-b", str(cfg.batch_size),
    ]
    run(cmd)

    candidates = (
        glob.glob(str(RVC_DIR / "weights" / f"{cfg.model}.pth"))
        + glob.glob(str(RVC_DIR / "logs" / cfg.model / "*.pth"))
    )
    if not candidates:
        raise FileNotFoundError("no .pth file found after training")

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    output_path = OUTPUT_DIR / f"{cfg.model}.pth"
    shutil.copy(candidates[0], output_path)
    print(f"[train] model saved: {output_path} ({output_path.stat().st_size} bytes)")
    return output_path


def upload_model(cfg: TrainConfig, model_path: Path, elapsed_secs: float):
    """Upload trained model to a GitHub release."""
    if not cfg.upload_token:
        print("[upload] GITHUB_TOKEN not set, skipping")
        return

    api = f"https://api.github.com/repos/{cfg.upload_repo}"
    auth = f"Authorization: token {cfg.upload_token}"
    tag = cfg.upload_tag
    body = json.dumps({
        "tag_name": tag,
        "name": f"{cfg.model} model",
        "body": f"epochs={cfg.epochs} batch={cfg.batch_size} time={elapsed_secs:.0f}s",
    })

    # Delete existing release (if any)
    subprocess.run(
        ["curl", "-sX", "DELETE", "-H", auth, f"{api}/releases/tags/{tag}"],
        capture_output=True,
    )

    # Create release
    result = subprocess.run(
        ["curl", "-s", "-H", auth, "-H", "Content-Type: application/json", f"{api}/releases", "-d", body],
        capture_output=True, text=True,
    )
    release = json.loads(result.stdout)
    upload_url = release.get("upload_url", "").replace("{?name,label}", "")

    if not upload_url:
        print(f"[upload] failed to create release: {result.stdout[:200]}")
        return

    # Upload asset
    name = model_path.name
    subprocess.run(
        ["curl", "-s", "-H", auth, "-H", "Content-Type: application/octet-stream",
         f"{upload_url}?name={name}", "--data-binary", f"@{model_path}"],
        capture_output=True,
    )
    print(f"[upload] {name} → {cfg.upload_repo}/releases/{tag}")


def mark_complete():
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUTPUT_DIR / "COMPLETE").write_text("ok")


def mark_failed(reason: str):
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUTPUT_DIR / "FAILED").write_text(reason)


def main():
    cfg = TrainConfig.from_env()
    print(f"[main] model={cfg.model} epochs={cfg.epochs} batch={cfg.batch_size}")

    try:
        install_rvc()
        wavs = download_dataset(cfg.dataset_url)

        t0 = time.time()
        model_path = train(cfg, wavs)
        elapsed = time.time() - t0
        print(f"[main] training completed in {elapsed:.0f}s ({elapsed / 60:.1f} min)")

        upload_model(cfg, model_path, elapsed)
        mark_complete()
        print("[main] DONE")

    except Exception as e:
        print(f"[main] FAILED: {e}")
        import traceback
        traceback.print_exc()
        mark_failed(str(e))
        sys.exit(1)


if __name__ == "__main__":
    main()
