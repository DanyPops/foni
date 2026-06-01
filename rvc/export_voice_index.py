"""Export FAISS voice index vectors to .npy for Rust k-NN blending.
Run: uv run --with faiss-cpu --with numpy python3 rvc/export_voice_index.py
"""
import faiss
import numpy as np
import pathlib

MODELS = [
    ("bandit",     "german-bandit-fp32-comp-de_esser-no_noise-norm.index"),
    ("sidorovich", "added_IVF591_Flat_nprobe_1_Sidorovich_v2.index"),
]

base_dir = pathlib.Path(__file__).parent / "models"

for model_name, idx_filename in MODELS:
    idx_path = base_dir / model_name / idx_filename
    out_path = base_dir / model_name / "voice_index_vectors.npy"

    if not idx_path.exists():
        print(f"skip {model_name} — {idx_path} not found")
        continue

    idx = faiss.read_index(str(idx_path))
    vecs = idx.reconstruct_n(0, idx.ntotal)
    np.save(str(out_path), vecs)
    print(f"{model_name}: {vecs.shape} ({vecs.nbytes // 1024**2} MB) → {out_path.name}")
