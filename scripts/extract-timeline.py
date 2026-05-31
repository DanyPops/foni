#!/usr/bin/env python3
"""
scripts/extract-timeline.py — whisper-timestamped → timeline fixture JSON.

Produces a frozen reference timeline for a studio WAV file:
  { words: [{word, start_s, end_s, confidence}],
    pauses: [{start_s, end_s, duration_s}],
    total_duration_s }

Run once per reference WAV; commit the output as a fixture.

Usage:
  python3 scripts/extract-timeline.py baseline/stalker/wav/sidorovich/trader1a.wav
  → baseline/stalker/timeline/trader1a.json
"""

import json
import sys
import os
from pathlib import Path

import whisper_timestamped as whisper

PAUSE_MIN_S = 0.05   # gaps shorter than 50ms are not counted as pauses
MODEL_SIZE  = "base"

def extract(wav_path: str) -> dict:
    audio  = whisper.load_audio(wav_path)
    model  = whisper.load_model(MODEL_SIZE, device="cpu")
    result = whisper.transcribe(model, audio, language="ru", verbose=False)

    words = []
    for seg in result["segments"]:
        for w in seg.get("words", []):
            words.append({
                "word":       w["text"].strip(),
                "start_s":    round(w["start"], 4),
                "end_s":      round(w["end"],   4),
                "confidence": round(w.get("confidence", 0.0), 4),
            })

    # Derive pauses from gaps between consecutive word boundaries
    pauses = []
    for i in range(len(words) - 1):
        gap_start = words[i]["end_s"]
        gap_end   = words[i + 1]["start_s"]
        duration  = round(gap_end - gap_start, 4)
        if duration >= PAUSE_MIN_S:
            pauses.append({
                "after_word": words[i]["word"],
                "start_s":    round(gap_start, 4),
                "end_s":      round(gap_end, 4),
                "duration_s": duration,
            })

    import soundfile as sf
    total_duration_s = sf.info(wav_path).duration

    return {
        "_source":        os.path.basename(wav_path),
        "_model":         MODEL_SIZE,
        "total_duration_s": round(total_duration_s, 4),
        "words":          words,
        "pauses":         pauses,
    }

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python3 scripts/extract-timeline.py <wav_file>", file=sys.stderr)
        sys.exit(1)

    wav_path = sys.argv[1]
    out_dir  = Path("baseline/stalker/timeline")
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / (Path(wav_path).stem + ".json")

    print(f"Transcribing {wav_path} with whisper-timestamped ({MODEL_SIZE})...")
    data = extract(wav_path)

    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)

    print(f"Written: {out_path}")
    print(f"  Words:  {len(data['words'])}")
    print(f"  Pauses: {len(data['pauses'])}")
    for w in data["words"]:
        print(f"    {w['word']:20s} {w['start_s']:.3f}–{w['end_s']:.3f}s  conf={w['confidence']:.2f}")
    print()
    for p in data["pauses"]:
        print(f"    [pause after '{p['after_word']}']  {p['duration_s']*1000:.0f}ms")
