"""
One-time adapter: export SpeechBrain ECAPA-TDNN to ONNX.

Output: rvc/models/pretrained/ecapa-voxceleb.onnx
Input:  16 kHz mono float32 [1, T] where T is waveform length
Output: speaker embedding [1, 192]

Run via: just export-ecapa
"""

import sys
import os
import torch
import torchaudio

OUT = "rvc/models/pretrained/ecapa-voxceleb.onnx"


def main():
    try:
        from speechbrain.inference.speaker import EncoderClassifier
    except ImportError:
        print("[export-ecapa] pip install speechbrain first", file=sys.stderr)
        sys.exit(1)

    print("[export-ecapa] loading ECAPA-TDNN from speechbrain/spkrec-ecapa-voxceleb...")
    model = EncoderClassifier.from_hparams(
        source="speechbrain/spkrec-ecapa-voxceleb",
        savedir="/tmp/ecapa-voxceleb",
        run_opts={"device": "cpu"},
    )
    model.eval()

    # SpeechBrain EncoderClassifier wraps the embedding model.
    # We export the inner model that maps [1, T] -> [1, 192].
    inner = model.mods.embedding_model
    inner.eval()

    # Representative input: 3 seconds at 16 kHz
    dummy = torch.randn(1, 48000)

    # SpeechBrain expects FBank features, not raw waveform. We need the full pipeline.
    # Wrap encode_batch logic into an exportable module.
    # Export ONLY the embedding model — skip the FBank/STFT (ONNX can't do complex STFT).
    # Rust will compute mel features and pass them in.
    embedding_model = model.mods.embedding_model
    embedding_model.eval()

    # Compute features for a dummy input to get the shape
    with torch.no_grad():
        feats = model.mods.compute_features(dummy)
        feats = model.mods.mean_var_norm(feats, torch.ones(feats.shape[0]))
        print(f"[export-ecapa] feature shape: {feats.shape}")

    os.makedirs(os.path.dirname(OUT), exist_ok=True)

    print(f"[export-ecapa] exporting to {OUT}...")
    with torch.no_grad():
        torch.onnx.export(
            embedding_model,
            feats,
            OUT,
            input_names=["features"],
            output_names=["embedding"],
            dynamic_axes={"features": {0: "batch", 1: "time"}},
            opset_version=17,
        )

    print(f"[export-ecapa] done — {os.path.getsize(OUT) / 1e6:.1f} MB")

    # Quick sanity check
    import onnxruntime as ort
    sess = ort.InferenceSession(OUT)
    out = sess.run(None, {"wav": dummy.numpy()})
    emb = out[0]
    print(f"[export-ecapa] embedding shape: {emb.shape}  norm: {(emb**2).sum()**0.5:.3f}")


if __name__ == "__main__":
    main()
