use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;

use crate::state::AppState;

/// Wire-compatible with the TS client which expects `{ models: string[] }`.
/// `onnx_ready` is an additive field — old clients ignore it.
#[derive(Serialize)]
pub struct ModelsResponse {
    /// All model names that have a `.pth` checkpoint.
    pub models: Vec<String>,
    /// Subset of `models` with `onnx/generator.onnx` present (ready for `/convert`).
    pub onnx_ready: Vec<String>,
}

pub async fn list(State(state): State<AppState>) -> Json<ModelsResponse> {
    let dir = &state.0.models_dir;

    let names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
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
        .collect();

    let onnx_ready = names
        .iter()
        .filter(|n| dir.join(n).join("onnx").join("generator.onnx").exists())
        .cloned()
        .collect();

    Json(ModelsResponse {
        models: names,
        onnx_ready,
    })
}

/// Select the active model by name.
///
/// ONNX sessions are loaded on-demand per `/convert` request — no pre-loading
/// needed here. This endpoint records the selection so `/convert` knows which
/// model directory to use.
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

    if onnx_ready {
        // Pre-warm sessions so the first /convert is instant.
        if let Err(e) = crate::sessions::ensure(&state, &name).await {
            tracing::warn!("session pre-load failed: {e}");
        }
    }

    Ok(Json(serde_json::json!({
        "model":      name,
        "onnx_ready": onnx_ready,
    })))
}
