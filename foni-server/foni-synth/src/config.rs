/// foni-rvc.yaml parser + environment-variable override layer.
///
/// Resolution order (highest wins):
///   1. Environment variables   (RVC_MODELS_DIR, FONI_SYNTH_ADDR, …)
///   2. foni-rvc.yaml           (searched in CWD, then repo-root rvc/)
///   3. Built-in defaults       (project-relative rvc/models, port 5050)
use std::path::PathBuf;

use serde::Deserialize;

use crate::state::RvcParams;

// ─── YAML schema ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
struct YamlConfig {
    model: Option<String>,
    models_dir: Option<String>,
    #[serde(default)]
    params: YamlParams,
}

#[derive(Debug, Deserialize, Default)]
struct YamlParams {
    f0method: Option<String>,
    f0up_key: Option<i32>,
    index_rate: Option<f32>,
    filter_radius: Option<u32>,
    rms_mix_rate: Option<f32>,
    protect: Option<f32>,
}

// ─── Resolved config ──────────────────────────────────────────────────────────

pub struct ServerConfig {
    pub models_dir: PathBuf,
    pub initial_model: Option<String>,
    pub params: RvcParams,
    /// Bind address, e.g. "0.0.0.0:5050".
    pub addr: String,
}

impl ServerConfig {
    pub fn load() -> Self {
        let yaml = load_yaml();

        // models_dir: env > yaml > workspace-relative default
        let models_dir = std::env::var("RVC_MODELS_DIR")
            .ok()
            .or_else(|| yaml.models_dir.clone())
            .map(PathBuf::from)
            .unwrap_or_else(Self::default_models_dir);

        // initial model: env > yaml
        let initial_model = std::env::var("RVC_MODEL").ok().or(yaml.model);

        // params: merge yaml over defaults, env vars take precedence
        let mut params = RvcParams::default();
        if let Some(v) = yaml.params.f0method {
            params.f0method = v;
        }
        if let Some(v) = yaml.params.f0up_key {
            params.f0up_key = v;
        }
        if let Some(v) = yaml.params.index_rate {
            params.index_rate = v;
        }
        if let Some(v) = yaml.params.filter_radius {
            params.filter_radius = v;
        }
        if let Some(v) = yaml.params.rms_mix_rate {
            params.rms_mix_rate = v;
        }
        if let Some(v) = yaml.params.protect {
            params.protect = v;
        }

        let addr = std::env::var("FONI_SYNTH_ADDR").unwrap_or_else(|_| "0.0.0.0:5050".into());

        Self {
            models_dir,
            initial_model,
            params,
            addr,
        }
    }

    /// Locate the project's rvc/models directory relative to the binary's CWD.
    /// Walks up until it finds a directory containing rvc/models/.
    fn default_models_dir() -> PathBuf {
        let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        for _ in 0..6 {
            let candidate = dir.join("rvc").join("models");
            if candidate.is_dir() {
                return candidate;
            }
            if !dir.pop() {
                break;
            }
        }
        // Last-resort container path (kept for Docker deployments)
        PathBuf::from("/app/rvc_models")
    }
}

// ─── YAML loading ─────────────────────────────────────────────────────────────

fn load_yaml() -> YamlConfig {
    for candidate in yaml_search_paths() {
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            match serde_yaml::from_str::<YamlConfig>(&text) {
                Ok(cfg) => {
                    tracing::info!("loaded config from {}", candidate.display());
                    return cfg;
                }
                Err(e) => {
                    tracing::warn!("bad foni-rvc.yaml at {}: {e}", candidate.display());
                }
            }
        }
    }
    tracing::info!("no foni-rvc.yaml found — using defaults");
    YamlConfig::default()
}

/// Candidate locations for foni-rvc.yaml, in priority order.
fn yaml_search_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("foni-rvc.yaml"),
        PathBuf::from("rvc/foni-rvc.yaml"),
    ];
    // Walk up from CWD
    if let Ok(mut dir) = std::env::current_dir() {
        for _ in 0..6 {
            paths.push(dir.join("rvc").join("foni-rvc.yaml"));
            paths.push(dir.join("foni-rvc.yaml"));
            if !dir.pop() {
                break;
            }
        }
    }
    paths
}
