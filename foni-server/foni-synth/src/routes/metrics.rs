//! GET /metrics — pool utilization and latency percentiles.
use axum::{extract::State, Json};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct Metrics {
    pub pool_size: usize,
    pub pool_active: usize,
    pub pool_waiting: usize,
    pub requests_total: u64,
    pub cache_hits: u64,
    pub cache_entries: usize,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
}

pub async fn get_metrics(State(state): State<AppState>) -> Json<Metrics> {
    use std::sync::atomic::Ordering;

    let pool = &state.0.sessions;
    let (p50, p95, p99) = pool.percentiles().await;

    Json(Metrics {
        pool_size: pool.size,
        pool_active: pool.active.load(Ordering::Relaxed),
        pool_waiting: pool.waiting.load(Ordering::Relaxed),
        requests_total: pool.requests_total.load(Ordering::Relaxed),
        cache_hits: pool.cache_hits.load(Ordering::Relaxed),
        cache_entries: state.cache_len().await,
        p50_ms: p50,
        p95_ms: p95,
        p99_ms: p99,
    })
}
