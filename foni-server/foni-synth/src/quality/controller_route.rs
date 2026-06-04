use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;

use super::dsp::controller::ControllerConfig;
use crate::config::{BreaksConfig, DspDefaults, EspeakConfig};
use crate::state::AppState;

/// GET /controller — current config + enabled state.
pub async fn get_controller(State(state): State<AppState>) -> Json<serde_json::Value> {
    let dsp = state
        .0
        .dsp_enabled
        .load(std::sync::atomic::Ordering::Relaxed);
    let ctrl = state
        .0
        .controller_enabled
        .load(std::sync::atomic::Ordering::Relaxed);
    let cfg = state.0.controller_config.read().await;
    let espeak = state.0.espeak_config.read().await;
    let breaks = state.0.breaks_config.read().await;
    let dsp_defaults = state.0.dsp_defaults.read().await;
    Json(serde_json::json!({
        "dsp": dsp,
        "controller": ctrl,
        "damping": cfg.damping,
        "targets": cfg.targets,
        "sensitivity": cfg.sensitivity,
        "ranges": cfg.ranges,
        "espeak": *espeak,
        "breaks": *breaks,
        "dsp_defaults": *dsp_defaults,
    }))
}

#[derive(Deserialize)]
pub struct ControllerUpdate {
    pub dsp: Option<bool>,
    pub enabled: Option<bool>,
    pub damping: Option<f32>,
    pub espeak: Option<EspeakConfig>,
    pub breaks: Option<BreaksConfig>,
    pub dsp_defaults: Option<DspDefaults>,
    pub targets: Option<super::dsp::controller::ControllerTargets>,
    pub sensitivity: Option<super::dsp::controller::ControllerSensitivity>,
    pub ranges: Option<super::dsp::controller::ControllerRanges>,
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
        // Reload policy script
        let new_policy = super::dsp::policy::find_policy_script()
            .and_then(|p| super::dsp::policy::PolicyEngine::load(&p))
            .map(std::sync::Arc::new);
        let had_policy = state.0.policy_engine.read().await.is_some();
        let has_policy = new_policy.is_some();
        *state.0.policy_engine.write().await = new_policy;
        return Ok(Json(serde_json::json!({
            "status": "reloaded from disk",
            "policy": if has_policy { "loaded" } else { "none" }
        })));
    }

    if let Some(dsp) = req.dsp {
        state
            .0
            .dsp_enabled
            .store(dsp, std::sync::atomic::Ordering::Relaxed);
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
    drop(cfg);

    if let Some(e) = req.espeak {
        *state.0.espeak_config.write().await = e;
    }
    if let Some(b) = req.breaks {
        *state.0.breaks_config.write().await = b;
    }
    if let Some(d) = req.dsp_defaults {
        *state.0.dsp_defaults.write().await = d;
    }

    let enabled = state
        .0
        .controller_enabled
        .load(std::sync::atomic::Ordering::Relaxed);

    Ok(Json(serde_json::json!({
        "status": "ok",
        "enabled": enabled,
    })))
}
