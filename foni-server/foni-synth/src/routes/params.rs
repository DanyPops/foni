use crate::state::{AppState, RvcParams};
use axum::{extract::State, Json};

pub async fn get_params(State(state): State<AppState>) -> Json<RvcParams> {
    Json(state.0.params.read().await.clone())
}

pub async fn set_params(
    State(state): State<AppState>,
    Json(patch): Json<serde_json::Value>,
) -> Json<RvcParams> {
    let mut p = state.0.params.write().await;
    // Merge only provided fields
    if let Some(v) = patch.get("f0up_key").and_then(|v| v.as_i64()) {
        p.f0up_key = v as i32;
    }
    if let Some(v) = patch.get("index_rate").and_then(|v| v.as_f64()) {
        p.index_rate = v as f32;
    }
    if let Some(v) = patch.get("protect").and_then(|v| v.as_f64()) {
        p.protect = v as f32;
    }
    if let Some(v) = patch.get("rms_mix_rate").and_then(|v| v.as_f64()) {
        p.rms_mix_rate = v as f32;
    }
    if let Some(v) = patch.get("f0method").and_then(|v| v.as_str()) {
        p.f0method = v.to_string();
    }
    Json(p.clone())
}
