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
    class EcapaWrapper(torch.nn.Module):
        def __init__(self, encoder_classifier):
            super().__init__()
            self.compute_features = encoder_classifier.mods.compute_features
            self.mean_var_norm = encoder_classifier.mods.mean_var_norm
            self.embedding_model = encoder_classifier.mods.embedding_model

        def forward(self, wav: torch.Tensor) -> torch.Tensor:
            """wav: [1, T] float32 at 16 kHz -> embedding [1, 192]"""
            feats = self.compute_features(wav)
            feats = self.mean_var_norm(feats, torch.ones(feats.shape[0]))
            return self.embedding_model(feats)

    wrapper = EcapaWrapper(model)
    wrapper.eval()

    os.makedirs(os.path.dirname(OUT), exist_ok=True)

    print(f"[export-ecapa] exporting to {OUT}...")
    with torch.no_grad():
        torch.onnx.export(
            wrapper,
            dummy,
            OUT,
            input_names=["wav"],
            output_names=["embedding"],
            dynamic_axes={"wav": {1: "T"}},
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
