//! Single config load point for the entire server.
//!
//! Resolution order (highest wins):
//!   1. Environment variables   (`FONI_` prefix, e.g. `FONI_CONTROLLER.DAMPING=0.5`)
//!   2. `rvc/dsp-defaults.json` (controller targets, sensitivity, ranges)
//!   3. `rvc/foni-rvc.yaml`     (model, params)
//!   4. Built-in defaults

use std::path::PathBuf;

use figment::providers::{Env, Format, Json, Serialized, Yaml};
use figment::Figment;
use serde::{Deserialize, Serialize};

use crate::dsp::controller::ControllerConfig;
use crate::state::RvcParams;

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ServerConfig {
    pub models_dir: String,
    pub model: Option<String>,
    #[serde(default)]
    pub params: RvcParams,
    pub controller: ControllerConfig,
    pub addr: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            models_dir: default_models_dir().to_string_lossy().into_owned(),
            model: None,
            params: RvcParams::default(),
            controller: ControllerConfig::default(),
            addr: "0.0.0.0:5050".into(),
        }
    }
}

/// The resolved config with typed paths.
pub struct ResolvedConfig {
    pub models_dir: PathBuf,
    pub initial_model: Option<String>,
    pub params: RvcParams,
    pub controller: ControllerConfig,
    pub addr: String,
}

impl ServerConfig {
    pub fn load() -> ResolvedConfig {
        let figment = Figment::new()
            .merge(Serialized::defaults(ServerConfig::default()))
            .merge(Yaml::file(find_yaml()))
            .merge(Json::file(find_json()))
            .merge(Env::prefixed("FONI_").split("_"));

        let cfg: ServerConfig = figment.extract().unwrap_or_else(|e| {
            tracing::warn!("config extraction error: {e} — using defaults");
            ServerConfig::default()
        });

        let models_dir = std::env::var("RVC_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(&cfg.models_dir));

        let initial_model = std::env::var("RVC_MODEL").ok().or(cfg.model);
        let addr = std::env::var("FONI_SYNTH_ADDR").unwrap_or(cfg.addr);

        tracing::info!(
            "config: models_dir={}, model={:?}, controller.enabled={}, addr={}",
            models_dir.display(),
            initial_model,
            cfg.controller.enabled,
            addr
        );

        ResolvedConfig {
            models_dir,
            initial_model,
            params: cfg.params,
            controller: cfg.controller,
            addr,
        }
    }
}

fn find_yaml() -> PathBuf {
    search_up("rvc/foni-rvc.yaml")
        .or_else(|| search_up("foni-rvc.yaml"))
        .unwrap_or_else(|| PathBuf::from("foni-rvc.yaml"))
}

fn find_json() -> PathBuf {
    search_up("rvc/dsp-defaults.json")
        .or_else(|| search_up("dsp-defaults.json"))
        .unwrap_or_else(|| PathBuf::from("dsp-defaults.json"))
}

fn search_up(name: &str) -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    for _ in 0..6 {
        let path = dir.join(name);
        if path.exists() {
            return Some(path);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn default_models_dir() -> PathBuf {
    search_up("rvc/models").unwrap_or_else(|| PathBuf::from("/app/rvc_models"))
}
