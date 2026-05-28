"""
Foni RVC Facade — config-driven voice conversion API.

Reads foni-rvc.yaml on startup, loads the configured model and params.
Exposes the same surface as rvc-python's built-in API server so the
TypeScript extension needs no changes.
"""

import base64
import os
import sys
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any

import yaml
from fastapi import FastAPI, HTTPException
from fastapi.responses import Response
from pydantic import BaseModel
from rvc_python.infer import RVCInference

# ─── Config ──────────────────────────────────────────────────────────────────

CONFIG_PATH = Path(os.environ.get("FONI_RVC_CONFIG", "/config/foni-rvc.yaml"))
MODELS_DIR = Path(os.environ.get("RVC_MODELS_DIR", "/app/rvc_models"))


def load_config() -> dict[str, Any]:
    if CONFIG_PATH.exists():
        with CONFIG_PATH.open() as f:
            return yaml.safe_load(f) or {}
    return {}


# ─── State ───────────────────────────────────────────────────────────────────

rvc = RVCInference(device="cpu")
current_model: str | None = None
current_params: dict[str, Any] = {}


def apply_params(params: dict[str, Any]) -> None:
    global current_params
    current_params = {**current_params, **params}
    if params.get("f0method"):
        rvc.f0method = params["f0method"]
    if "f0up_key" in params:
        rvc.f0up_key = params["f0up_key"]
    if "index_rate" in params:
        rvc.index_rate = params["index_rate"]
    if "filter_radius" in params:
        rvc.filter_radius = params["filter_radius"]
    if "resample_sr" in params:
        rvc.resample_sr = params["resample_sr"]
    if "rms_mix_rate" in params:
        rvc.rms_mix_rate = params["rms_mix_rate"]
    if "protect" in params:
        rvc.protect = params["protect"]


def load_model(name: str) -> None:
    global current_model
    model_dir = MODELS_DIR / name
    pth_files = list(model_dir.glob("*.pth"))
    index_files = list(model_dir.glob("*.index"))
    if not pth_files:
        raise FileNotFoundError(f"No .pth file in {model_dir}")
    rvc.load_model(
        str(pth_files[0]),
        index_path=str(index_files[0]) if index_files else None,
    )
    current_model = name
    print(f"[foni-rvc] model loaded: {name} ({pth_files[0].name})", flush=True)


# ─── Startup ─────────────────────────────────────────────────────────────────

@asynccontextmanager
async def lifespan(app: FastAPI):
    cfg = load_config()
    models_dir_override = cfg.get("models_dir")
    global MODELS_DIR
    if models_dir_override:
        MODELS_DIR = Path(models_dir_override)

    if params := cfg.get("params"):
        apply_params(params)
        print(f"[foni-rvc] params applied: {params}", flush=True)

    if model := cfg.get("model"):
        try:
            load_model(model)
        except Exception as e:
            print(f"[foni-rvc] warning: could not load model '{model}': {e}", flush=True)

    print("[foni-rvc] ready", flush=True)
    yield


# ─── API ─────────────────────────────────────────────────────────────────────

app = FastAPI(lifespan=lifespan)


class ConvertRequest(BaseModel):
    audio_data: str


class ParamsRequest(BaseModel):
    params: dict[str, Any]


class DeviceRequest(BaseModel):
    device: str


class ModelsDirRequest(BaseModel):
    models_dir: str


@app.get("/models")
def list_models() -> dict:
    models = [
        d.name
        for d in MODELS_DIR.iterdir()
        if d.is_dir() and any(d.glob("*.pth"))
    ] if MODELS_DIR.exists() else []
    return {"models": sorted(models)}


@app.post("/models/{name}")
def load_model_endpoint(name: str) -> dict:
    try:
        load_model(name)
        return {"message": f"Model {name} loaded successfully"}
    except FileNotFoundError as e:
        raise HTTPException(status_code=404, detail=str(e))
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


@app.get("/params")
def get_params() -> dict:
    return {
        "f0method": getattr(rvc, "f0method", "rmvpe"),
        "f0up_key": getattr(rvc, "f0up_key", 0),
        "index_rate": getattr(rvc, "index_rate", 0.6),
        "filter_radius": getattr(rvc, "filter_radius", 3),
        "resample_sr": getattr(rvc, "resample_sr", 0),
        "rms_mix_rate": getattr(rvc, "rms_mix_rate", 0.25),
        "protect": getattr(rvc, "protect", 0.5),
        "current_model": current_model,
    }


@app.post("/params")
def set_params(req: ParamsRequest) -> dict:
    apply_params(req.params)
    return {"message": "params updated", "params": current_params}


@app.post("/set_models_dir")
def set_models_dir(req: ModelsDirRequest) -> dict:
    global MODELS_DIR
    MODELS_DIR = Path(req.models_dir)
    return {"message": f"Models directory set to {MODELS_DIR}"}


@app.post("/set_device")
def set_device(req: DeviceRequest) -> dict:
    rvc.device = req.device
    return {"message": f"Device set to {req.device}"}


@app.post("/convert")
def convert(req: ConvertRequest) -> Response:
    if current_model is None:
        raise HTTPException(status_code=503, detail="No model loaded")
    try:
        audio_bytes = base64.b64decode(req.audio_data)
        input_path = "/tmp/foni_input.wav"
        output_path = "/tmp/foni_output.wav"
        with open(input_path, "wb") as f:
            f.write(audio_bytes)
        rvc.infer_file(input_path, output_path)
        with open(output_path, "rb") as f:
            result = f.read()
        return Response(content=result, media_type="audio/wav")
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


if __name__ == "__main__":
    import uvicorn
    port = int(os.environ.get("RVC_PORT", "5050"))
    uvicorn.run(app, host="0.0.0.0", port=port)
