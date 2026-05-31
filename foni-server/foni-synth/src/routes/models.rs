use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct ModelInfo {
    pub name: String,
    /// True when onnx/generator.onnx exists alongside the .pth checkpoint.
    pub onnx_ready: bool,
}

#[derive(Serialize)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

pub async fn list(State(state): State<AppState>) -> Json<ModelsResponse> {
    let dir = &state.0.models_dir;
    let models = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        // Only directories that contain a .pth checkpoint (skip pretrained/, etc.)
        .filter(|e| {
            e.path()
                .read_dir()
                .map(|mut d| {
                    d.any(|f| {
                        f.map(|f| f.path().extension().map(|x| x == "pth").unwrap_or(false))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .map(|name| {
            let onnx_ready = dir.join(&name).join("onnx").join("generator.onnx").exists();
            ModelInfo { name, onnx_ready }
        })
        .collect();

    Json(ModelsResponse { models })
}

/// Select the active model by name.
///
/// ONNX sessions are loaded on-demand per `/convert` request — no pre-loading
/// needed here. This endpoint simply records the selection in shared state so
/// that `/convert` and `/params` know which model directory to use.
pub async fn load(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let model_path = state.0.models_dir.join(&name);
    if !model_path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("model '{name}' not found")));
    }

    let onnx_ready = model_path.join("onnx").join("generator.onnx").exists();
    *state.0.current_model.write().await = Some(name.clone());

    Ok(Json(serde_json::json!({
        "model":      name,
        "onnx_ready": onnx_ready,
    })))
}
