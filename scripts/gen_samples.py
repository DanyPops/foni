#!/usr/bin/env python3
"""
Generate listen samples through the full foni pipeline:
  espeak → /convert (ContentVec + RMVPE + Generator) → /process (DSP)

Output: samples/ directory with WAV files + a reference (original game audio).
"""

import base64, json, subprocess, sys, urllib.request, urllib.error, shutil
from pathlib import Path

BASE      = "http://localhost:5051"
SAMPLES   = Path(__file__).parent.parent / "samples"
VOICE     = "ru"
SPEED     = 150   # espeak WPM
MODEL     = "bandit"

PHRASES = [
    ("01_trader1a",   "Подойди-ка, надо тебе ситуацию прояснить."),
    ("02_greeting",   "Привет, сталкер. Как дела на болотах?"),
    ("03_warning",    "Осторожно. Здесь аномалии, не зевай."),
    ("04_deal",       "Деплой прошёл успешно, коммиты запушены."),
    ("05_farewell",   "Удачи, браток. На Зоне удача нужна."),
]


def post_json(path: str, payload: dict, out_file: Path) -> bool:
    data = json.dumps(payload).encode()
    req  = urllib.request.Request(
        f"{BASE}{path}", data=data,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            body = resp.read()
            out_file.write_bytes(body)
            return True
    except urllib.error.HTTPError as e:
        print(f"  HTTP {e.code}: {e.read()[:200]}", file=sys.stderr)
        return False


def espeak_wav(text: str, out: Path) -> bool:
    r = subprocess.run(
        ["espeak-ng", "-v", VOICE, "-s", str(SPEED), "-w", str(out), text],
        capture_output=True,
    )
    return r.returncode == 0 and out.exists()


def b64(path: Path) -> str:
    return base64.b64encode(path.read_bytes()).decode()


def main():
    SAMPLES.mkdir(exist_ok=True)

    # Select model
    req = urllib.request.Request(f"{BASE}/models/{MODEL}",
                                  data=b"", method="POST")
    urllib.request.urlopen(req, timeout=5)
    print(f"Model: {MODEL}")

    # Reference — original game WAV (no processing)
    ref_src  = Path("baseline/stalker/wav/sidorovich/trader1a.wav")
    ref_dest = SAMPLES / "00_reference_original.wav"
    if ref_src.exists():
        shutil.copy(ref_src, ref_dest)
        print(f"✅ Reference copied → {ref_dest.name}")
    else:
        print(f"⚠  Reference not found at {ref_src}")

    for slug, phrase in PHRASES:
        print(f"\n── {slug}: «{phrase[:40]}»")

        raw_wav = SAMPLES / f"{slug}_a_espeak.wav"
        rvc_wav = SAMPLES / f"{slug}_b_rvc_raw.wav"
        dsp_wav = SAMPLES / f"{slug}_c_rvc_dsp.wav"

        # 1. Espeak raw
        if not espeak_wav(phrase, raw_wav):
            print("  ❌ espeak failed"); continue
        print(f"  ✅ espeak     → {raw_wav.name}  ({raw_wav.stat().st_size//1024}kB)")

        # 2. /convert — RVC voice conversion (raw, no DSP)
        ok = post_json("/convert",
                       {"audio_data": b64(raw_wav), "model": MODEL},
                       rvc_wav)
        if not ok: print("  ❌ /convert failed"); continue
        print(f"  ✅ /convert   → {rvc_wav.name}  ({rvc_wav.stat().st_size//1024}kB)")

        # 3. /process — Rust DSP chain on top of RVC output
        ok = post_json("/process",
                       {
                           "audio_data": b64(rvc_wav),
                           "opts": {
                               "rmsTargetLufs":       -8,
                               "compressionRatio":     4,
                               "compressionMakeupDb":  5,
                               "tiltLowDb":           10,
                               "tiltHighDb":          -8,
                               "vibratoFreq":          6,
                               "vibratoDepth":        0.003,
                           },
                       },
                       dsp_wav)
        if not ok: print("  ❌ /process failed"); continue
        print(f"  ✅ /process   → {dsp_wav.name}  ({dsp_wav.stat().st_size//1024}kB)")

    print(f"\n── Done.  Files in: {SAMPLES.resolve()}")
    print("\nListen order (in samples/):")
    print("  00_reference_original.wav   ← Sidorovich studio recording")
    for slug, phrase in PHRASES:
        print(f"  {slug}_a_espeak.wav          ← espeak only (no RVC)")
        print(f"  {slug}_b_rvc_raw.wav         ← RVC (no DSP)")
        print(f"  {slug}_c_rvc_dsp.wav         ← RVC + full DSP chain  ← this is production")


if __name__ == "__main__":
    main()
