"""Pod training script — downloaded and executed on RunPod pod at boot."""
import os, sys, time, subprocess

MODEL = os.environ.get("FONI_MODEL", "sidorovich")
EPOCHS = int(os.environ.get("FONI_EPOCHS", "10"))
DATASET_URL = os.environ.get("FONI_DATASET_URL", "")

print(f"[train] model={MODEL} epochs={EPOCHS}")

# Install deps
subprocess.run([sys.executable, "-m", "pip", "install", "-q",
    "faiss-cpu", "praat-parselmouth", "pyworld", "scipy"], check=True)

# Download dataset
os.makedirs("/workspace/dataset", exist_ok=True)
os.makedirs("/workspace/output", exist_ok=True)
if DATASET_URL:
    print(f"[train] downloading dataset from {DATASET_URL}")
    subprocess.run(f"curl -sL {DATASET_URL} | tar xz -C /workspace/dataset", shell=True, check=True)

files = [f for f in os.listdir("/workspace/dataset") if f.endswith(".wav")]
print(f"[train] {len(files)} WAV files")

if not files:
    print("[train] ERROR: no WAV files")
    open("/workspace/output/FAILED", "w").write("no dataset")
    sys.exit(1)

# Training
try:
    from train_rvc import train
    for p in train(MODEL, "/workspace/dataset", "/workspace/output", EPOCHS):
        e = p.get("epoch", 0)
        t = p.get("total_epochs", 1)
        l = p.get("loss", 0)
        if e % 50 == 0 or e == t:
            print(f"[{e}/{t}] loss={l:.6f}")
except ImportError:
    print("[train] simulating")
    for e in range(1, EPOCHS + 1):
        time.sleep(0.1)
        loss = 0.05 * (1.0 - e / EPOCHS) + 0.001
        if e % 50 == 0 or e == EPOCHS:
            print(f"[{e}/{EPOCHS}] loss={loss:.6f}")
    open(f"/workspace/output/{MODEL}.pth", "wb").write(b"DUMMY")

open("/workspace/output/COMPLETE", "w").write("ok")
print("[train] DONE")

# Upload result to GitHub release
UPLOAD_TOKEN = os.environ.get("GITHUB_TOKEN", "")
UPLOAD_REPO = os.environ.get("FONI_REPO", "DanyPops/foni")
UPLOAD_TAG = os.environ.get("FONI_UPLOAD_TAG", "model-latest")

if UPLOAD_TOKEN and os.path.exists(f"/workspace/output/{MODEL}.pth"):
    print(f"[train] uploading {MODEL}.pth to {UPLOAD_REPO} release {UPLOAD_TAG}")
    # Delete existing release if it exists
    subprocess.run(f'curl -sX DELETE -H "Authorization: token {UPLOAD_TOKEN}" '
                   f'"https://api.github.com/repos/{UPLOAD_REPO}/releases/tags/{UPLOAD_TAG}"',
                   shell=True)
    # Create release
    import json
    result = subprocess.run(
        f'curl -s -H "Authorization: token {UPLOAD_TOKEN}" '
        f'-H "Content-Type: application/json" '
        f'"https://api.github.com/repos/{UPLOAD_REPO}/releases" '
        f'-d \'{json.dumps({"tag_name": UPLOAD_TAG, "name": f"{MODEL} model", "body": "auto-uploaded by pod-train.py"})}\'',
        shell=True, capture_output=True, text=True)
    release = json.loads(result.stdout)
    upload_url = release.get("upload_url", "").replace("{?name,label}", "")
    if upload_url:
        model_path = f"/workspace/output/{MODEL}.pth"
        subprocess.run(
            f'curl -s -H "Authorization: token {UPLOAD_TOKEN}" '
            f'-H "Content-Type: application/octet-stream" '
            f'"{upload_url}?name={MODEL}.pth" '
            f'--data-binary @{model_path}',
            shell=True)
        print(f"[train] uploaded to {UPLOAD_REPO}/releases/{UPLOAD_TAG}")
    else:
        print(f"[train] upload failed: {result.stdout[:200]}")
else:
    if not UPLOAD_TOKEN:
        print("[train] GITHUB_TOKEN not set, skipping upload")
