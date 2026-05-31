#!/usr/bin/env python3
"""
export_contentvec_onnx.py — export ContentVec (HuBERT-based) to ONNX.

Downloads the IAHispano/Applio contentvec model (HubertModelWithFinalProj)
and exports it to:
    rvc/models/pretrained/contentvec-768-l12.onnx

Input:  [1, 1, T]   float32  (16 kHz audio, ~0.5–3 s → T ≈ 8000–48000)
Output: [1, T', 256] float32  (phone features, T' = T / 320 approx)

The T' dimension is the phone length passed to the generator.

Usage:
    python3 rvc/export_contentvec_onnx.py
"""

import os
import sys
import urllib.request
import torch
import torch.nn as nn
from transformers import HubertModel, HubertConfig

PRETRAINED_DIR = os.path.join(os.path.dirname(__file__), "models", "pretrained")
ONNX_PATH      = os.path.join(PRETRAINED_DIR, "contentvec-768-l12.onnx")
CONFIG_URL     = "https://huggingface.co/IAHispano/Applio/resolve/main/Resources/embedders/contentvec/config.json"
WEIGHTS_URL    = "https://huggingface.co/IAHispano/Applio/resolve/main/Resources/embedders/contentvec/pytorch_model.bin"
CONFIG_PATH    = os.path.join(PRETRAINED_DIR, "contentvec_config.json")
WEIGHTS_PATH   = os.path.join(PRETRAINED_DIR, "contentvec_weights.bin")


class ContentVecExtractor(HubertModel):
    """Extract raw 768-dim features from HuBERT layer 12.

    The bandit generator expects phone inputs of shape [1, T, 768].
    We do NOT apply final_proj (which would reduce to 256-dim).
    """
    def __init__(self, config: HubertConfig):
        super().__init__(config)
        # final_proj is in the checkpoint — load it but don't use it
        self.final_proj = nn.Linear(config.hidden_size, config.classifier_proj_size)

    def forward(self, input_values: torch.Tensor) -> torch.Tensor:  # type: ignore[override]
        # input_values: [1, T]
        out = super().forward(input_values, output_hidden_states=True)
        # Layer-12 hidden states: [1, T', 768]
        return out.hidden_states[12]


def download(url: str, path: str) -> None:
    if os.path.exists(path):
        print(f"  cached: {path}")
        return
    print(f"  downloading {url} → {path}")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    urllib.request.urlretrieve(url, path)
    size_mb = os.path.getsize(path) / 1024 / 1024
    print(f"  done ({size_mb:.1f} MB)")


def main() -> None:
    print("ContentVec ONNX export")
    print("======================")

    # 1. Download config + weights
    download(CONFIG_URL,  CONFIG_PATH)
    download(WEIGHTS_URL, WEIGHTS_PATH)

    # 2. Load model
    print("Loading model …")
    config = HubertConfig.from_json_file(CONFIG_PATH)
    model  = ContentVecExtractor(config)

    state = torch.load(WEIGHTS_PATH, map_location="cpu")
    missing, unexpected = model.load_state_dict(state, strict=False)
    if missing:
        print(f"  missing keys:    {missing[:5]}")
    if unexpected:
        print(f"  unexpected keys: {unexpected[:5]}")

    model.eval()

    # 3. Dummy input — 1 s at 16 kHz
    T = 16_000
    dummy_flat  = torch.zeros(1, T)                  # [1, T] for forward()
    dummy_outer = torch.zeros(1, 1, T)               # [1, 1, T] for ONNX wrapper

    # Verify forward pass
    with torch.no_grad():
        out = model(dummy_flat)
    print(f"  forward OK → shape {list(out.shape)}")   # [1, T', 256]

    # 4. ONNX wrapper: accepts [1, 1, T], squeezes dim 1 before passing to model
    class ContentVecWrapper(nn.Module):
        def __init__(self, inner: ContentVecExtractor):
            super().__init__()
            self.inner = inner

        def forward(self, x: torch.Tensor) -> torch.Tensor:  # x: [1, 1, T]
            return self.inner(x.squeeze(1))                  # → [1, T', 768]

    wrapper = ContentVecWrapper(model)
    with torch.no_grad():
        out2 = wrapper(dummy_outer)
    print(f"  wrapper OK → shape {list(out2.shape)}")

    # 5. Export
    print(f"Exporting to {ONNX_PATH} …")
    os.makedirs(PRETRAINED_DIR, exist_ok=True)

    torch.onnx.export(
        wrapper,
        dummy_outer,
        ONNX_PATH,
        input_names=["source"],
        output_names=["embed"],
        dynamic_axes={"source": {2: "T"}, "embed": {1: "T_prime"}},
        do_constant_folding=True,
        opset_version=17,
        dynamo=False,
    )

    size_mb = os.path.getsize(ONNX_PATH) / 1024 / 1024
    print(f"  exported ({size_mb:.1f} MB)")

    # 6. Validate with onnxruntime
    print("Validating …")
    import onnxruntime as ort
    import numpy as np

    sess = ort.InferenceSession(ONNX_PATH, providers=["CPUExecutionProvider"])
    inp  = np.zeros((1, 1, T), dtype=np.float32)
    out3 = sess.run(None, {"source": inp})[0]
    print(f"  onnxruntime output shape: {out3.shape}")
    assert out3.ndim == 3, "expected 3-dim output"
    assert out3.shape[0] == 1
    assert out3.shape[2] == 768, f"expected 768 features, got {out3.shape[2]}"
    print("✅ ContentVec ONNX export validated")


if __name__ == "__main__":
    main()
