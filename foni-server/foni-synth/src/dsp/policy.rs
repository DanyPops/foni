//! Rhai-based acoustic policy engine.
//!
//! Loads a `.rhai` script that defines a `correct(measured, target, knobs, damping)` function.
//! Hot-reloadable: edit the script, POST /controller {reload: true}, new policy is live.
//!
//! Falls back to the compiled proportional controller if no script is found.

use rhai::{Dynamic, Engine, AST};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::dsp::SmoothingOptions;
use foni_analyse::AnalysisResult;

use super::controller::{ControllerConfig, ControllerSnapshot};

pub struct PolicyEngine {
    engine: Engine,
    ast: AST,
    path: PathBuf,
}

impl PolicyEngine {
    pub fn load(path: &Path) -> Option<Self> {
        if !path.exists() {
            tracing::info!(
                "no policy script at {} — using compiled controller",
                path.display()
            );
            return None;
        }

        let mut engine = Engine::new();
        engine.set_max_expr_depths(64, 32);

        match engine.compile_file(path.into()) {
            Ok(ast) => {
                tracing::info!("policy script loaded from {}", path.display());
                Some(PolicyEngine {
                    engine,
                    ast,
                    path: path.to_path_buf(),
                })
            }
            Err(e) => {
                tracing::warn!("failed to compile policy script {}: {e}", path.display());
                None
            }
        }
    }

    pub fn reload(&mut self) -> bool {
        match self.engine.compile_file(self.path.clone()) {
            Ok(ast) => {
                self.ast = ast;
                tracing::info!("policy script reloaded from {}", self.path.display());
                true
            }
            Err(e) => {
                tracing::warn!("failed to reload policy script: {e}");
                false
            }
        }
    }

    pub fn evaluate(
        &self,
        analysis: &AnalysisResult,
        base: &SmoothingOptions,
        cfg: &ControllerConfig,
    ) -> Option<(SmoothingOptions, ControllerSnapshot)> {
        let measured = rhai::Map::from([
            (
                "brightness_hz".into(),
                Dynamic::from(analysis.spectral.brightness_hz),
            ),
            (
                "loudness_db".into(),
                Dynamic::from(analysis.loudness.rms_db),
            ),
            (
                "bass_balance_db".into(),
                Dynamic::from(analysis.spectral.bass_balance_db),
            ),
            (
                "vocal_darkness_db_oct".into(),
                Dynamic::from(analysis.spectral.vocal_darkness_db_oct),
            ),
            (
                "breathiness_db".into(),
                Dynamic::from(analysis.voice.breathiness_db),
            ),
            (
                "voice_presence".into(),
                Dynamic::from(analysis.pitch.voice_presence),
            ),
        ]);

        let target = rhai::Map::from([
            (
                "brightness_hz".into(),
                Dynamic::from(cfg.targets.brightness_hz),
            ),
            ("loudness_db".into(), Dynamic::from(cfg.targets.loudness_db)),
            (
                "bass_balance_db".into(),
                Dynamic::from(cfg.targets.bass_balance_db),
            ),
            (
                "vocal_darkness_db_oct".into(),
                Dynamic::from(cfg.targets.vocal_darkness_db_oct),
            ),
        ]);

        let knobs = rhai::Map::from([
            ("tilt_high_db".into(), Dynamic::from(base.tilt_high_db)),
            ("tilt_low_db".into(), Dynamic::from(base.tilt_low_db)),
            (
                "rms_target_lufs".into(),
                Dynamic::from(base.rms_target_lufs),
            ),
            ("presence_db".into(), Dynamic::from(base.presence_db)),
            ("de_harsh_db".into(), Dynamic::from(base.de_harsh_db)),
            (
                "compression_ratio".into(),
                Dynamic::from(base.compression_ratio),
            ),
        ]);

        let damping = Dynamic::from(cfg.damping);

        let mut scope = rhai::Scope::new();
        let result: rhai::Map = self
            .engine
            .call_fn(
                &mut scope,
                &self.ast,
                "correct",
                (measured.clone(), target, knobs, damping),
            )
            .map_err(|e| tracing::warn!("policy script error: {e}"))
            .ok()?;

        let get_f32 = |map: &rhai::Map, key: &str, fallback: f32| -> f32 {
            map.get(key)
                .and_then(|v| v.as_float().ok().map(|f| f as f32))
                .unwrap_or(fallback)
        };

        let mut opts = base.clone();
        opts.tilt_high_db = get_f32(&result, "tilt_high_db", base.tilt_high_db);
        opts.tilt_low_db = get_f32(&result, "tilt_low_db", base.tilt_low_db);
        opts.rms_target_lufs = get_f32(&result, "rms_target_lufs", base.rms_target_lufs);
        opts.presence_db = get_f32(&result, "presence_db", base.presence_db);
        opts.de_harsh_db = get_f32(&result, "de_harsh_db", base.de_harsh_db);
        opts.compression_ratio = get_f32(&result, "compression_ratio", base.compression_ratio);

        let m = &measured;
        let snap = ControllerSnapshot {
            enabled: true,
            measured_brightness_hz: get_f32(m, "brightness_hz", 0.0),
            measured_loudness_db: get_f32(m, "loudness_db", 0.0),
            measured_bass_balance_db: get_f32(m, "bass_balance_db", 0.0),
            measured_vocal_darkness: get_f32(m, "vocal_darkness_db_oct", 0.0),
            target_brightness_hz: cfg.targets.brightness_hz,
            target_loudness_db: cfg.targets.loudness_db,
            target_bass_balance_db: cfg.targets.bass_balance_db,
            target_vocal_darkness: cfg.targets.vocal_darkness_db_oct,
            correction_tilt_low_db: opts.tilt_low_db - base.tilt_low_db,
            correction_tilt_high_db: opts.tilt_high_db - base.tilt_high_db,
            correction_rms_lufs: opts.rms_target_lufs - base.rms_target_lufs,
            correction_presence_db: opts.presence_db - base.presence_db,
            correction_de_harsh_db: opts.de_harsh_db - base.de_harsh_db,
        };

        Some((opts, snap))
    }
}

/// Search for acoustic-policy.rhai in standard locations.
pub fn find_policy_script() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("rvc/acoustic-policy.rhai"),
        PathBuf::from("acoustic-policy.rhai"),
    ];
    for p in &candidates {
        if p.exists() {
            return Some(p.clone());
        }
    }
    if let Ok(mut dir) = std::env::current_dir() {
        for _ in 0..6 {
            let p = dir.join("rvc").join("acoustic-policy.rhai");
            if p.exists() {
                return Some(p);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    None
}
