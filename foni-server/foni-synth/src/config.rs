//! Single config load point for the entire server.
//!
//! Resolution order (highest wins):
//!   1. Environment variables   (`FONI_` prefix, e.g. `FONI_CONTROLLER.DAMPING=0.5`)
//!   2. `training/dsp-defaults.json` (controller targets, sensitivity, ranges)
//!   3. `training/foni-rvc.yaml`     (model, params)
//!   4. Built-in defaults

use std::path::PathBuf;

use figment::providers::{Env, Format, Json, Serialized, Yaml};
use figment::Figment;
use serde::{Deserialize, Serialize};

use crate::quality::dsp::controller::ControllerConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BreaksConfig {
    pub comma: u32,
    pub semicolon: u32,
    pub colon: u32,
    pub dash: u32,
    pub ellipsis: u32,
    pub period: u32,
    pub exclamation: u32,
    pub question: u32,
}

impl Default for BreaksConfig {
    fn default() -> Self {
        Self {
            comma: 150,
            semicolon: 220,
            colon: 180,
            dash: 200,
            ellipsis: 420,
            period: 320,
            exclamation: 300,
            question: 350,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DspDefaults {
    pub tilt_low_db: f32,
    pub tilt_high_db: f32,
    pub rms_target_lufs: f32,
    pub compression_ratio: f32,
    pub compression_attack_ms: f32,
    pub compression_release_ms: f32,
    pub compression_threshold_db: f32,
    pub compression_makeup_db: f32,
    pub presence_db: f32,
    pub de_ess_db: f32,
    pub de_harsh_db: f32,
    pub de_harsh_freq: f32,
    pub de_harsh_q: f32,
    pub vibrato_freq: f32,
    pub vibrato_depth: f32,
    pub highpass_freq: f32,
    pub warmth_boost_db: f32,
    pub warmth_freq: f32,
    pub air_boost_db: f32,
    pub air_freq: f32,
    pub reverb_ms: f32,
    pub reverb_decay: f32,
    pub reverb_in_gain: f32,
    pub reverb_out_gain: f32,
    pub fade_secs: f32,
    pub limiter_db: f32,
}

impl Default for DspDefaults {
    fn default() -> Self {
        Self {
            tilt_low_db: 0.0,
            tilt_high_db: 0.0,
            rms_target_lufs: -16.0,
            compression_ratio: 2.0,
            compression_attack_ms: 10.0,
            compression_release_ms: 80.0,
            compression_threshold_db: -12.0,
            compression_makeup_db: 2.0,
            presence_db: 0.0,
            de_ess_db: 2.0,
            de_harsh_db: -2.0,
            de_harsh_freq: 3500.0,
            de_harsh_q: 0.7,
            vibrato_freq: 0.0,
            vibrato_depth: 0.0,
            highpass_freq: 80.0,
            warmth_boost_db: 0.0,
            warmth_freq: 200.0,
            air_boost_db: 0.0,
            air_freq: 8000.0,
            reverb_ms: 6.0,
            reverb_decay: 0.03,
            reverb_in_gain: 0.8,
            reverb_out_gain: 0.88,
            fade_secs: 0.04,
            limiter_db: -1.0,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ServerConfig {
    pub models_dir: String,
    pub model: Option<String>,
    #[serde(default)]
    pub params: RvcParams,
    pub controller: ControllerConfig,
    pub breaks: BreaksConfig,
    pub dsp: DspDefaults,
    pub addr: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            models_dir: default_models_dir().to_string_lossy().into_owned(),
            model: None,
            params: RvcParams::default(),
            controller: ControllerConfig::default(),
            breaks: BreaksConfig::default(),
            dsp: DspDefaults::default(),
            addr: "0.0.0.0:5050".into(),
        }
    }
}

pub struct ResolvedConfig {
    pub models_dir: PathBuf,
    pub initial_model: Option<String>,
    pub params: RvcParams,
    pub controller: ControllerConfig,
    pub breaks: BreaksConfig,
    pub dsp: DspDefaults,
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
            breaks: cfg.breaks,
            dsp: cfg.dsp,
            addr,
        }
    }
}

fn find_yaml() -> PathBuf {
    search_up("training/foni-rvc.yaml")
        .or_else(|| search_up("foni-rvc.yaml"))
        .unwrap_or_else(|| PathBuf::from("foni-rvc.yaml"))
}

fn find_json() -> PathBuf {
    search_up("training/dsp-defaults.json")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaks_default_all_positive() {
        let b = BreaksConfig::default();
        assert!(b.comma > 0);
        assert!(b.period > 0);
        assert!(b.ellipsis > b.comma);
    }

    #[test]
    fn server_config_default_has_valid_addr() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.addr, "0.0.0.0:5050");
    }
}

/// Voice conversion parameters (legacy RVC, will be replaced by Fish Speech config).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RvcParams {
    pub f0up_key: i32,
    pub index_rate: f32,
    pub filter_radius: u32,
    pub rms_mix_rate: f32,
    pub protect: f32,
    pub f0method: String,
}

impl Default for RvcParams {
    fn default() -> Self {
        RvcParams {
            f0up_key: -2,
            index_rate: 0.77,
            filter_radius: 5,
            rms_mix_rate: 0.45,
            protect: 0.33,
            f0method: "rmvpe".to_string(),
        }
    }
}
