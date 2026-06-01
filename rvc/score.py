"""
Acoustic quality scorer — Tier B metrics.

Outputs JSON:
  utmos         : float   — predicted MOS 1-5 (UTMOSv2, higher = more natural)
  ecapa_sim     : float   — cosine similarity vs Sidorovich corpus mean (0-1)
  ecapa_sim_pct : float   — ecapa_sim as percentage

Usage:
  python3 rvc/score.py audio.wav [--reference-dir baseline/stalker/wav/sidorovich/]
  python3 rvc/score.py --dir synth_dir/ [--reference-dir ...]
"""

import argparse
import json
import os
import sys
from pathlib import Path

import numpy as np
import torch
import torchaudio


def load_mono_16k(path: str) -> torch.Tensor:
    wav, sr = torchaudio.load(path)
    if wav.shape[0] > 1:
        wav = wav.mean(dim=0, keepdim=True)
    if sr != 16000:
        wav = torchaudio.functional.resample(wav, sr, 16000)
    return wav


def ecapa_embed(wav_16k: torch.Tensor, classifier) -> np.ndarray:
    with torch.no_grad():
        length = torch.ones(1)
        emb = classifier.encode_batch(wav_16k, length)
    return emb.squeeze().cpu().numpy()


def cosine_sim(a: np.ndarray, b: np.ndarray) -> float:
    a_n = a / (np.linalg.norm(a) + 1e-8)
    b_n = b / (np.linalg.norm(b) + 1e-8)
    return float(np.dot(a_n, b_n))


def build_corpus_mean(ref_dir: str, classifier) -> np.ndarray:
    wavs = sorted(Path(ref_dir).glob("*.wav"))
    if not wavs:
        print(f"[score] no WAVs in {ref_dir}", file=sys.stderr)
        return None
    embeds = []
    for p in wavs:
        wav = load_mono_16k(str(p))
        embeds.append(ecapa_embed(wav, classifier))
    mean = np.stack(embeds).mean(axis=0)
    return mean / (np.linalg.norm(mean) + 1e-8)


def score_one(wav_path: str, classifier, corpus_mean, utmos_predictor) -> dict:
    result = {"file": wav_path}

    if utmos_predictor is not None:
        result["utmos"] = round(float(utmos_predictor.predict(wav_path)), 3)
    else:
        result["utmos"] = None

    if classifier is not None:
        wav = load_mono_16k(wav_path)
        emb = ecapa_embed(wav, classifier)
        if corpus_mean is not None:
            sim = cosine_sim(emb, corpus_mean)
            result["ecapa_sim"] = round(sim, 4)
            result["ecapa_sim_pct"] = round(sim * 100, 1)
        else:
            result["ecapa_sim"] = None
            result["ecapa_sim_pct"] = None

    return result


def main():
    parser = argparse.ArgumentParser(description="Tier B acoustic scorer")
    parser.add_argument("wav", nargs="?", help="WAV file to score")
    parser.add_argument("--dir", help="Score all WAVs in this directory")
    parser.add_argument("--reference-dir", help="Sidorovich corpus dir for ECAPA corpus mean")
    parser.add_argument("--no-utmos", action="store_true", help="Skip UTMOSv2 (faster)")
    parser.add_argument("--no-ecapa", action="store_true", help="Skip ECAPA (faster)")
    parser.add_argument("--save-corpus-mean", help="Save corpus mean embedding to .npy file")
    args = parser.parse_args()

    if not args.wav and not args.dir:
        parser.error("Provide a WAV file or --dir")

    utmos_predictor = None
    if not args.no_utmos:
        try:
            import utmosv2
            print("[score] loading UTMOSv2...", file=sys.stderr)
            utmos_predictor = utmosv2.create_predictor()
        except ImportError:
            print("[score] utmosv2 not installed — skipping MOS prediction", file=sys.stderr)

    classifier = None
    corpus_mean = None
    if not args.no_ecapa:
        try:
            from speechbrain.inference.speaker import EncoderClassifier
            print("[score] loading ECAPA-TDNN...", file=sys.stderr)
            classifier = EncoderClassifier.from_hparams(
                source="speechbrain/spkrec-ecapa-voxceleb",
                savedir="/tmp/ecapa-voxceleb",
                run_opts={"device": "cpu"},
            )
        except ImportError:
            print("[score] speechbrain not installed — skipping ECAPA", file=sys.stderr)

        if classifier is not None and args.reference_dir:
            print(f"[score] building corpus mean from {args.reference_dir}...", file=sys.stderr)
            corpus_mean = build_corpus_mean(args.reference_dir, classifier)
            if corpus_mean is not None and args.save_corpus_mean:
                np.save(args.save_corpus_mean, corpus_mean)
                print(f"[score] corpus mean saved → {args.save_corpus_mean}", file=sys.stderr)

    if args.dir:
        wavs = sorted(Path(args.dir).glob("*.wav"))
        results = [score_one(str(p), classifier, corpus_mean, utmos_predictor) for p in wavs]
        results.sort(key=lambda r: r.get("ecapa_sim") or 0.0, reverse=True)
        print(json.dumps(results, indent=2))

        print("\n── Ranked by ECAPA similarity ──────────────────", file=sys.stderr)
        for r in results[:10]:
            utmos = f"MOS={r['utmos']:.2f}" if r.get("utmos") is not None else "MOS=n/a"
            ecapa = f"ECAPA={r['ecapa_sim_pct']:.1f}%" if r.get("ecapa_sim_pct") is not None else "ECAPA=n/a"
            print(f"  {utmos}  {ecapa}  {Path(r['file']).name}", file=sys.stderr)
    else:
        result = score_one(args.wav, classifier, corpus_mean, utmos_predictor)
        print(json.dumps(result, indent=2))

        print("\n── Tier B metrics ──────────────────────────────", file=sys.stderr)
        if result.get("utmos") is not None:
            print(f"  UTMOSv2 MOS  : {result['utmos']:.3f}  (target: >3.5)", file=sys.stderr)
        if result.get("ecapa_sim") is not None:
            bar = "█" * int(result["ecapa_sim"] * 20)
            print(f"  ECAPA sim    : {result['ecapa_sim_pct']:.1f}%  {bar}  (target: >70%)", file=sys.stderr)


if __name__ == "__main__":
    main()
