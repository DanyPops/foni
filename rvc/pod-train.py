"""Pod training script — downloaded and executed on RunPod pod at boot."""
import os, sys, time, subprocess, glob, json

MODEL = os.environ.get("FONI_MODEL", "sidorovich")
EPOCHS = int(os.environ.get("FONI_EPOCHS", "500"))
BATCH_SIZE = int(os.environ.get("FONI_BATCH_SIZE", "16"))
DATASET_URL = os.environ.get("FONI_DATASET_URL", "")

print(f"[train] model={MODEL} epochs={EPOCHS} batch={BATCH_SIZE}")

# Install rvc-no-gui
if not os.path.exists("/workspace/rvc-no-gui"):
    print("[train] cloning rvc-no-gui...")
    subprocess.run(
        ["git", "clone", "--depth=1",
         "https://github.com/nakshatra-garg/rvc-no-gui.git",
         "/workspace/rvc-no-gui"],
        check=True)

os.chdir("/workspace/rvc-no-gui")
subprocess.run([sys.executable, "-m", "pip", "install", "-q", "-r", "requirements.txt"])

# Setup RVC (downloads pretrained models)
print("[train] setting up RVC pretrained models...")
subprocess.run([sys.executable, "pipeline.py", "setup"], check=True)

# Download dataset
os.makedirs("/workspace/dataset", exist_ok=True)
os.makedirs("/workspace/output", exist_ok=True)
if DATASET_URL:
    print(f"[train] downloading dataset from {DATASET_URL}")
    subprocess.run(
        f"curl -sL {DATASET_URL} | tar xzf - --no-same-owner -C /workspace/dataset",
        shell=True)

files = glob.glob("/workspace/dataset/*.wav")
print(f"[train] {len(files)} WAV files")

if not files:
    print("[train] ERROR: no WAV files")
    open("/workspace/output/FAILED", "w").write("no dataset")
    sys.exit(1)

# Train
print(f"[train] starting training: {EPOCHS} epochs, batch={BATCH_SIZE}")
t0 = time.time()

try:
    from pipeline import RVCPipeline
    from pathlib import Path
    from config import PipelineConfig

    config = PipelineConfig()
    config.training.epochs = EPOCHS
    config.training.batch_size = BATCH_SIZE
    config.f0.method = "rmvpe_gpu"

    pipeline = RVCPipeline(config)
    pipeline.run_full_training(
        model_name=MODEL,
        audio_files=[Path(f) for f in files],
        epochs=EPOCHS,
        batch_size=BATCH_SIZE,
        skip_setup=True,
    )
    elapsed = time.time() - t0
    print(f"[train] training complete in {elapsed:.0f}s")

    # Find the trained model
    model_paths = (
        glob.glob(f"weights/{MODEL}.pth") +
        glob.glob(f"logs/{MODEL}/*.pth")
    )
    if model_paths:
        import shutil
        shutil.copy(model_paths[0], f"/workspace/output/{MODEL}.pth")
        print(f"[train] model saved: /workspace/output/{MODEL}.pth")
    else:
        print("[train] WARNING: no .pth file found after training")

except Exception as e:
    print(f"[train] ERROR: {e}")
    import traceback
    traceback.print_exc()
    open("/workspace/output/FAILED", "w").write(str(e))
    sys.exit(1)

# Upload result to GitHub release
UPLOAD_TOKEN = os.environ.get("GITHUB_TOKEN", "")
UPLOAD_REPO = os.environ.get("FONI_REPO", "DanyPops/foni")
UPLOAD_TAG = os.environ.get("FONI_UPLOAD_TAG", "model-latest")

model_file = f"/workspace/output/{MODEL}.pth"
if UPLOAD_TOKEN and os.path.exists(model_file):
    print(f"[train] uploading {MODEL}.pth to {UPLOAD_REPO} release {UPLOAD_TAG}")
    subprocess.run(
        f'curl -sX DELETE -H "Authorization: token {UPLOAD_TOKEN}" '
        f'"https://api.github.com/repos/{UPLOAD_REPO}/releases/tags/{UPLOAD_TAG}"',
        shell=True)
    result = subprocess.run(
        f'curl -s -H "Authorization: token {UPLOAD_TOKEN}" '
        f'-H "Content-Type: application/json" '
        f'"https://api.github.com/repos/{UPLOAD_REPO}/releases" '
        f'-d \'{json.dumps({"tag_name": UPLOAD_TAG, "name": f"{MODEL} model", "body": f"epochs={EPOCHS} batch={BATCH_SIZE} time={elapsed:.0f}s"})}\'',
        shell=True, capture_output=True, text=True)
    release = json.loads(result.stdout)
    upload_url = release.get("upload_url", "").replace("{?name,label}", "")
    if upload_url:
        subprocess.run(
            f'curl -s -H "Authorization: token {UPLOAD_TOKEN}" '
            f'-H "Content-Type: application/octet-stream" '
            f'"{upload_url}?name={MODEL}.pth" '
            f'--data-binary @{model_file}',
            shell=True)
        print(f"[train] uploaded to {UPLOAD_REPO}/releases/{UPLOAD_TAG}")
    else:
        print(f"[train] upload failed: {result.stdout[:200]}")
else:
    if not UPLOAD_TOKEN:
        print("[train] GITHUB_TOKEN not set, skipping upload")

open("/workspace/output/COMPLETE", "w").write("ok")
print("[train] DONE")
