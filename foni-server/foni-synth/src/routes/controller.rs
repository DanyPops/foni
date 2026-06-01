use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;

use crate::dsp::controller::ControllerConfig;
use crate::state::AppState;

/// GET /controller — current config + enabled state.
pub async fn get_controller(State(state): State<AppState>) -> Json<serde_json::Value> {
    let enabled = state
        .0
        .controller_enabled
        .load(std::sync::atomic::Ordering::Relaxed);
    let cfg = state.0.controller_config.read().await;
    Json(serde_json::json!({
        "enabled": enabled,
        "damping": cfg.damping,
        "targets": cfg.targets,
        "sensitivity": cfg.sensitivity,
        "ranges": cfg.ranges,
    }))
}

#[derive(Deserialize)]
pub struct ControllerUpdate {
    pub enabled: Option<bool>,
    pub damping: Option<f32>,
    pub targets: Option<crate::dsp::controller::ControllerTargets>,
    pub sensitivity: Option<crate::dsp::controller::ControllerSensitivity>,
    pub ranges: Option<crate::dsp::controller::ControllerRanges>,
    pub reload: Option<bool>,
}

/// POST /controller — update config live. Any field omitted is left unchanged.
/// Pass `{"reload": true}` to re-read from dsp-defaults.json without restart.
pub async fn set_controller(
    State(state): State<AppState>,
    Json(req): Json<ControllerUpdate>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.reload == Some(true) {
        let fresh = crate::config::ServerConfig::load();
        *state.0.controller_config.write().await = fresh.controller.clone();
        state.0.controller_enabled.store(
            fresh.controller.enabled,
            std::sync::atomic::Ordering::Relaxed,
        );
        return Ok(Json(serde_json::json!({ "status": "reloaded from disk" })));
    }

    if let Some(enabled) = req.enabled {
        state
            .0
            .controller_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    let mut cfg = state.0.controller_config.write().await;
    if let Some(d) = req.damping {
        cfg.damping = d;
    }
    if let Some(t) = req.targets {
        cfg.targets = t;
    }
    if let Some(s) = req.sensitivity {
        cfg.sensitivity = s;
    }
    if let Some(r) = req.ranges {
        cfg.ranges = r;
    }

    let enabled = state
        .0
        .controller_enabled
        .load(std::sync::atomic::Ordering::Relaxed);

    Ok(Json(serde_json::json!({
        "status": "ok",
        "enabled": enabled,
        "damping": cfg.damping,
    })))
}
