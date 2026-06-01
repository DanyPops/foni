#!/usr/bin/env python3
"""
rvc/export_onnx.py — Export RVC v2 generator to ONNX for Rust inference via ort.

Pipeline:
  audio → ContentVec (vec-768-layer-12.onnx) → phone_features [1, T, 768]
  audio → RMVPE (rmvpe.onnx)                 → pitch [1, T], pitchf [1, T]
  phone_features + pitch → generator.onnx   → audio_out

Outputs:
  rvc/models/<name>/onnx/generator.onnx   — 106 MB, inputs: phone/pitch/rnd/ds
  rvc/models/pretrained/rmvpe.onnx        — 345 MB (downloaded)
  rvc/models/pretrained/vec-768-layer-12.onnx — ContentVec (requires fairseq to export)

ContentVec note:
  ContentVec ONNX requires `fairseq` which conflicts with faiss-cpu version pins.
  Workaround: use pre-exported ContentVec from HuggingFace when available, or
  run the Python RVC inference (via rvc_python) as a sidecar until ONNX is available.

Usage:
  python3 rvc/export_onnx.py bandit
  python3 rvc/export_onnx.py sidorovich

Requirements:
  pip install torch onnx onnxruntime onnxscript
  Clone: git clone --depth=1 --sparse https://github.com/RVC-Project/Retrieval-based-Voice-Conversion-WebUI /tmp/rvc-onnx-source
         cd /tmp/rvc-onnx-source && git sparse-checkout set tools infer/lib/infer_pack
"""

import sys
import os
import warnings
from pathlib import Path

import torch

MODELS_DIR  = Path(__file__).parent / "models"
PRETRAINED  = Path(__file__).parent / "models" / "pretrained"
RVC_SOURCE  = Path("/tmp/rvc-onnx-source")

# Patch for PyTorch 2.12+ compatibility
def _patch_attentions(src: Path) -> None:
    f = src / "infer/lib/infer_pack/attentions_onnx.py"
    text = f.read_text()
    text = text.replace(
        "pad_length = torch.clamp(length - (self.window_size + 1), min=0)",
        "pad_length = max(0, int(length) - (self.window_size + 1))",
    ).replace(
        "slice_start_position = torch.clamp((self.window_size + 1) - length, min=0)",
        "slice_start_position = max(0, (self.window_size + 1) - int(length))",
    ).replace(
        "slice_end_position = slice_start_position + 2 * length - 1",
        "slice_end_position = slice_start_position + 2 * int(length) - 1",
    )
    f.write_text(text)


def _ensure_rvc_source() -> None:
    """Clone the minimal RVC source tree if not present."""
    import subprocess
    if RVC_SOURCE.exists():
        return
    print("Cloning RVC source (sparse, depth=1)...")
    subprocess.run([
        "git", "clone", "--depth=1", "--filter=blob:none", "--sparse",
        "https://github.com/RVC-Project/Retrieval-based-Voice-Conversion-WebUI",
        str(RVC_SOURCE),
    ], check=True)
    subprocess.run(
        ["git", "sparse-checkout", "set", "infer/lib/infer_pack"],
        cwd=str(RVC_SOURCE), check=True,
    )
    print("  RVC source ready.")


def export_generator(model_name: str) -> Path:
    _ensure_rvc_source()

    sys.path.insert(0, str(RVC_SOURCE))
    _patch_attentions(RVC_SOURCE)
    from infer.lib.infer_pack.models_onnx import SynthesizerTrnMsNSFsidM

    model_dir = MODELS_DIR / model_name
    pth_files  = list(model_dir.glob("*.pth"))
    if not pth_files:
        raise FileNotFoundError(f"No .pth found in {model_dir}")

    pth = pth_files[0]
    print(f"Loading {pth.name}...")
    cpt = torch.load(str(pth), map_location="cpu", weights_only=False)
    cpt["config"][-3] = cpt["weight"]["emb_g.weight"].shape[0]
    version   = cpt.get("version", "v2")
    phone_dim = cpt["weight"]["enc_p.emb_phone.weight"].shape[1]

    print(f"  version={version}  phone_dim={phone_dim}")
    print(f"  config: {cpt['config']}")

    T    = 200
    args = (
        torch.rand(1, T, phone_dim),
        torch.tensor([T]).long(),
        torch.randint(size=(1, T), low=5, high=255),
        torch.rand(1, T),
        torch.LongTensor([0]),
        torch.rand(1, 192, T),
    )

    net_g = SynthesizerTrnMsNSFsidM(*cpt["config"], is_half=False, version=version)
    net_g.load_state_dict(cpt["weight"], strict=False)
    net_g.eval()

    # Forward check
    with torch.no_grad():
        out = net_g(*args)
    print(f"  Forward OK: {out.shape}")

    out_dir = model_dir / "onnx"
    out_dir.mkdir(exist_ok=True)
    out_path = out_dir / "generator.onnx"

    print("Exporting ONNX (TorchScript path, takes ~60s)...")
    import threading, itertools, time
    done = threading.Event()
    def spin():
        for c in itertools.cycle("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"):
            if done.is_set(): break
            print(f"  {c} exporting...", end="\r", flush=True)
            time.sleep(0.1)
    t = threading.Thread(target=spin, daemon=True)
    t.start()
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        torch.onnx.export(
            net_g, args, str(out_path),
            dynamic_axes={"phone": [1], "pitch": [1], "pitchf": [1], "rnd": [2]},
            do_constant_folding=False, opset_version=16, verbose=False,
            input_names=["phone","phone_lengths","pitch","pitchf","ds","rnd"],
            output_names=["audio"],
            dynamo=False,
        )

    done.set(); t.join()
    size_mb = os.path.getsize(out_path) / 1024 / 1024
    print(f"  Written: {out_path}  ({size_mb:.1f} MB)")

    # Validate
    import onnxruntime as ort
    import numpy as np
    sess = ort.InferenceSession(str(out_path))
    inp  = {
        "phone":         np.random.rand(1, T, phone_dim).astype(np.float32),
        "phone_lengths": np.array([T], dtype=np.int64),
        "pitch":         np.random.randint(5, 255, (1, T)).astype(np.int64),
        "pitchf":        np.random.rand(1, T).astype(np.float32),
        "ds":            np.array([0], dtype=np.int64),
        "rnd":           np.random.rand(1, 192, T).astype(np.float32),
    }
    audio = sess.run(None, inp)[0]
    print(f"  ONNX validated: audio {audio.shape}  dtype={audio.dtype}")
    return out_path


def main():
    model_name = sys.argv[1] if len(sys.argv) > 1 else "bandit"
    print(f"\n=== Exporting RVC generator: {model_name} ===")
    path = export_generator(model_name)
    print(f"\n✅ Generator: {path}")
    print("\nNote: ContentVec (vec-768-layer-12.onnx) requires fairseq to export.")
    print("      RMVPE available at: rvc/models/pretrained/rmvpe.onnx (download separately)")
    print("      Full inference pipeline: ContentVec → RMVPE → generator")


if __name__ == "__main__":
    main()
