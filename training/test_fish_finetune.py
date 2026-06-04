"""Tests for fish-finetune.py — run with: python3 -m pytest rvc/test_fish_finetune.py -v"""

import os
import tempfile
from pathlib import Path
from unittest.mock import patch

import pytest

_src = Path(__file__).parent / "fish-finetune.py"
_code = _src.read_text()
_module = {}
exec(compile(_code, _src, "exec"), _module)

FinetuneConfig = _module["FinetuneConfig"]


class TestFinetuneConfig:
    def test_defaults(self):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith("FONI_") and k != "GITHUB_TOKEN"}
        with patch.dict(os.environ, env, clear=True):
            cfg = FinetuneConfig.from_env()
            assert cfg.model == "sidorovich"
            assert cfg.steps == 100
            assert cfg.workspace == Path("/workspace")
            assert cfg.upload_tag == "model-sidorovich"

    def test_custom_env(self):
        env = {
            "FONI_MODEL": "test",
            "FONI_EPOCHS": "50",
            "FONI_WORKSPACE": "/tmp/test",
            "FONI_DATASET_URL": "https://example.com/data.tar.gz",
        }
        with patch.dict(os.environ, env, clear=False):
            cfg = FinetuneConfig.from_env()
            assert cfg.model == "test"
            assert cfg.steps == 50
            assert cfg.workspace == Path("/tmp/test")

    def test_paths(self):
        with patch.dict(os.environ, {"FONI_MODEL": "voice", "FONI_WORKSPACE": "/w"}, clear=False):
            cfg = FinetuneConfig.from_env()
            assert cfg.fish_dir == Path("/w/fish-speech")
            assert cfg.data_dir == Path("/w/data/voice")
            assert cfg.output_dir == Path("/w/output")


class TestDownloadDataset:
    def test_creates_lab_files(self):
        download_dataset = _module["download_dataset"]
        with tempfile.TemporaryDirectory() as tmp:
            cfg = FinetuneConfig(
                model="test", steps=1, dataset_url="",
                workspace=Path(tmp), upload_token="",
                upload_repo="", upload_tag="",
            )
            raw_dir = Path(tmp) / "dataset-raw"
            raw_dir.mkdir()
            (raw_dir / "a.wav").write_bytes(b"RIFF" + b"\x00" * 40)
            (raw_dir / "b.wav").write_bytes(b"RIFF" + b"\x00" * 40)
            (raw_dir / "transcripts.txt").write_text(
                "a.wav|Привет\nb.wav|Пока\n")

            count = download_dataset(cfg)
            assert count == 2
            assert (cfg.data_dir / "a.lab").read_text() == "Привет"
            assert (cfg.data_dir / "b.lab").read_text() == "Пока"
            assert (cfg.data_dir / "a.wav").exists()

    def test_skips_files_without_transcript(self):
        download_dataset = _module["download_dataset"]
        with tempfile.TemporaryDirectory() as tmp:
            cfg = FinetuneConfig(
                model="test", steps=1, dataset_url="",
                workspace=Path(tmp), upload_token="",
                upload_repo="", upload_tag="",
            )
            raw_dir = Path(tmp) / "dataset-raw"
            raw_dir.mkdir()
            (raw_dir / "a.wav").write_bytes(b"RIFF" + b"\x00" * 40)
            (raw_dir / "no_transcript.wav").write_bytes(b"RIFF" + b"\x00" * 40)
            (raw_dir / "transcripts.txt").write_text("a.wav|Hello\n")

            count = download_dataset(cfg)
            assert count == 1

    def test_raises_on_missing_transcripts(self):
        download_dataset = _module["download_dataset"]
        with tempfile.TemporaryDirectory() as tmp:
            cfg = FinetuneConfig(
                model="test", steps=1, dataset_url="",
                workspace=Path(tmp), upload_token="",
                upload_repo="", upload_tag="",
            )
            (Path(tmp) / "dataset-raw").mkdir()
            with pytest.raises(FileNotFoundError, match="transcripts.txt"):
                download_dataset(cfg)


class TestMarkers:
    def test_complete(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = FinetuneConfig(
                model="t", steps=1, dataset_url="",
                workspace=Path(tmp), upload_token="",
                upload_repo="", upload_tag="",
            )
            _module["mark_complete"](cfg)
            assert (cfg.output_dir / "COMPLETE").read_text() == "ok"

    def test_failed(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg = FinetuneConfig(
                model="t", steps=1, dataset_url="",
                workspace=Path(tmp), upload_token="",
                upload_repo="", upload_tag="",
            )
            _module["mark_failed"](cfg, "boom")
            assert (cfg.output_dir / "FAILED").read_text() == "boom"
