use axum::{extract::State, Json};

use crate::state::AppState;

/// DELETE /cache — flush the in-process WAV LRU cache.
pub async fn clear_cache(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut cache = state.0.wav_cache.lock().await;
    let count = cache.len();
    cache.clear();
    Json(serde_json::json!({"cleared": count}))
}
