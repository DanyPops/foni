"""Tests for pod-train.py — run with: python3 -m pytest rvc/test_pod_train.py -v"""

import os
import sys
import tempfile
from pathlib import Path
from collections import namedtuple
from unittest.mock import patch

import pytest

sys.path.insert(0, str(Path(__file__).parent))

_src = Path(__file__).parent / "pod-train.py"
_code = _src.read_text()
_module = {}
exec(compile(_code, _src, "exec"), _module)

TrainConfig = _module["TrainConfig"]

VersionInfo = namedtuple("version_info", ["major", "minor", "micro"])


class TestTrainConfig:
    def test_defaults(self):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith("FONI_") and k != "GITHUB_TOKEN"}
        with patch.dict(os.environ, env, clear=True):
            cfg = TrainConfig.from_env()
            assert cfg.model == "sidorovich"
            assert cfg.epochs == 500
            assert cfg.batch_size == 16
            assert cfg.upload_tag == "model-sidorovich"
            assert cfg.dataset_url == ""
            assert cfg.upload_token == ""

    def test_custom_env(self):
        env = {
            "FONI_MODEL": "test_voice",
            "FONI_EPOCHS": "100",
            "FONI_BATCH_SIZE": "32",
            "FONI_DATASET_URL": "https://example.com/data.tar.gz",
            "GITHUB_TOKEN": "ghp_fake",
            "FONI_REPO": "user/repo",
            "FONI_UPLOAD_TAG": "v2",
        }
        with patch.dict(os.environ, env, clear=False):
            cfg = TrainConfig.from_env()
            assert cfg.model == "test_voice"
            assert cfg.epochs == 100
            assert cfg.batch_size == 32
            assert cfg.dataset_url == "https://example.com/data.tar.gz"
            assert cfg.upload_token == "ghp_fake"
            assert cfg.upload_repo == "user/repo"
            assert cfg.upload_tag == "v2"

    def test_upload_tag_derives_from_model(self):
        with patch.dict(os.environ, {"FONI_MODEL": "mymodel"}, clear=False):
            os.environ.pop("FONI_UPLOAD_TAG", None)
            cfg = TrainConfig.from_env()
            assert cfg.upload_tag == "model-mymodel"


class TestSetupPython:
    def test_compatible_python_returns_sys_executable(self):
        setup_python = _module["setup_python"]
        with patch.object(sys, "version_info", VersionInfo(3, 10, 0)):
            result = setup_python()
            assert result == sys.executable

    def test_too_new_python_returns_conda_path(self):
        setup_python = _module["setup_python"]
        conda_python = str(_module["PYTHON"])
        with patch.object(sys, "version_info", VersionInfo(3, 12, 3)):
            with patch.object(Path, "exists", return_value=True):
                result = setup_python()
                assert result == conda_python


class TestDownloadDataset:
    def test_no_url_no_files_raises(self):
        download_dataset = _module["download_dataset"]
        with tempfile.TemporaryDirectory() as tmp:
            _module["DATASET_DIR"] = Path(tmp)
            with pytest.raises(FileNotFoundError, match="no WAV"):
                download_dataset("")

    def test_finds_existing_wavs(self):
        download_dataset = _module["download_dataset"]
        with tempfile.TemporaryDirectory() as tmp:
            _module["DATASET_DIR"] = Path(tmp)
            (Path(tmp) / "a.wav").write_bytes(b"fake")
            (Path(tmp) / "b.wav").write_bytes(b"fake")
            (Path(tmp) / "c.txt").write_bytes(b"not audio")
            wavs = download_dataset("")
            assert len(wavs) == 2
            assert all(str(w).endswith(".wav") for w in wavs)

    def test_wavs_are_sorted(self):
        download_dataset = _module["download_dataset"]
        with tempfile.TemporaryDirectory() as tmp:
            _module["DATASET_DIR"] = Path(tmp)
            (Path(tmp) / "z.wav").write_bytes(b"fake")
            (Path(tmp) / "a.wav").write_bytes(b"fake")
            wavs = download_dataset("")
            assert wavs[0].name == "a.wav"
            assert wavs[1].name == "z.wav"


class TestMarkers:
    def test_mark_complete(self):
        with tempfile.TemporaryDirectory() as tmp:
            _module["OUTPUT_DIR"] = Path(tmp)
            _module["mark_complete"]()
            assert (Path(tmp) / "COMPLETE").read_text() == "ok"

    def test_mark_failed(self):
        with tempfile.TemporaryDirectory() as tmp:
            _module["OUTPUT_DIR"] = Path(tmp)
            _module["mark_failed"]("disk full")
            assert (Path(tmp) / "FAILED").read_text() == "disk full"
