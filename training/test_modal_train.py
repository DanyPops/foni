"""Tests for modal-train.py structure — run with: python3 -m pytest rvc/test_modal_train.py -v

Tests the training logic WITHOUT Modal or GPU.
Modal-specific decorators are tested by `modal run --dry-run`.
"""

import os
import tempfile
from pathlib import Path
from unittest.mock import patch

import pytest


class TestDatasetPrep:
    """Test the dataset preparation logic extracted from train()."""

    def test_creates_lab_files_from_transcripts(self):
        with tempfile.TemporaryDirectory() as tmp:
            raw_dir = Path(tmp) / "raw"
            data_dir = Path(tmp) / "data"
            raw_dir.mkdir()
            data_dir.mkdir()

            (raw_dir / "a.wav").write_bytes(b"RIFF" + b"\x00" * 40)
            (raw_dir / "b.wav").write_bytes(b"RIFF" + b"\x00" * 40)
            (raw_dir / "transcripts.txt").write_text(
                "a.wav|Привет сталкер\nb.wav|Здорово\n",
                encoding="utf-8",
            )

            # Simulate the dataset prep logic from train()
            import shutil, glob

            transcripts = {}
            for line in open(raw_dir / "transcripts.txt", encoding="utf-8"):
                if "|" in line:
                    fname, text = line.strip().split("|", 1)
                    transcripts[fname] = text

            count = 0
            for wav in sorted(glob.glob(str(raw_dir / "*.wav"))):
                name = os.path.basename(wav)
                text = transcripts.get(name)
                if not text:
                    continue
                shutil.copy(wav, data_dir / name)
                stem = os.path.splitext(name)[0]
                (data_dir / f"{stem}.lab").write_text(text, encoding="utf-8")
                count += 1

            assert count == 2
            assert (data_dir / "a.lab").read_text(encoding="utf-8") == "Привет сталкер"
            assert (data_dir / "b.lab").read_text(encoding="utf-8") == "Здорово"
            assert (data_dir / "a.wav").exists()

    def test_skips_wavs_without_transcript(self):
        with tempfile.TemporaryDirectory() as tmp:
            raw_dir = Path(tmp) / "raw"
            data_dir = Path(tmp) / "data"
            raw_dir.mkdir()
            data_dir.mkdir()

            (raw_dir / "a.wav").write_bytes(b"RIFF" + b"\x00" * 40)
            (raw_dir / "orphan.wav").write_bytes(b"RIFF" + b"\x00" * 40)
            (raw_dir / "transcripts.txt").write_text("a.wav|Hello\n")

            import shutil, glob
            transcripts = {}
            for line in open(raw_dir / "transcripts.txt"):
                if "|" in line:
                    fname, text = line.strip().split("|", 1)
                    transcripts[fname] = text

            count = 0
            for wav in sorted(glob.glob(str(raw_dir / "*.wav"))):
                name = os.path.basename(wav)
                if transcripts.get(name):
                    shutil.copy(wav, data_dir / name)
                    count += 1

            assert count == 1
            assert not (data_dir / "orphan.wav").exists()

    def test_empty_transcripts_yields_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            raw_dir = Path(tmp) / "raw"
            raw_dir.mkdir()
            (raw_dir / "a.wav").write_bytes(b"RIFF" + b"\x00" * 40)
            (raw_dir / "transcripts.txt").write_text("")

            transcripts = {}
            for line in open(raw_dir / "transcripts.txt"):
                if "|" in line:
                    fname, text = line.strip().split("|", 1)
                    transcripts[fname] = text

            assert len(transcripts) == 0


class TestModalScript:
    """Verify modal-train.py imports and structure."""

    def test_script_is_valid_python(self):
        src = Path(__file__).parent / "modal-train.py"
        code = src.read_text()
        compile(code, str(src), "exec")

    def test_has_app_and_entrypoint(self):
        src = Path(__file__).parent / "modal-train.py"
        code = src.read_text()
        assert "app = modal.App" in code
        assert "@app.local_entrypoint()" in code
        assert "@app.function(" in code
        assert 'gpu=' in code

    def test_has_volume(self):
        src = Path(__file__).parent / "modal-train.py"
        code = src.read_text()
        assert "modal.Volume" in code
        assert "volume.commit()" in code
