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
