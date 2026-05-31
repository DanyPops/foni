use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde::Serialize;
use crate::state::AppState;

#[derive(Serialize)]
pub struct ModelsResponse { pub models: Vec<String> }

pub async fn list(State(state): State<AppState>) -> Json<ModelsResponse> {
    let dir = &state.0.models_dir;
    let models = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| e.path().read_dir().map(|mut d| d.any(|f| {
            f.map(|f| f.path().extension().map(|x| x == "pth").unwrap_or(false)).unwrap_or(false)
        })).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    Json(ModelsResponse { models })
}

pub async fn load(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let model_path = state.0.models_dir.join(&name);
    if !model_path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("model '{name}' not found")));
    }
    // TODO (FON-TSK-11): load ONNX model via ort
    *state.0.current_model.write().await = Some(name.clone());
    Ok(Json(serde_json::json!({ "message": format!("Model {name} loaded") })))
}
