//! GET /metrics — cache stats and synthesis info.
use axum::{extract::State, Json};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct Metrics {
    pub cache_size: usize,
}

pub async fn metrics(State(state): State<AppState>) -> Json<Metrics> {
    Json(Metrics {
        cache_size: state.cache_len().await,
    })
}
