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
                Dynamic::from(analysis.spectral.brightness_hz as f64),
            ),
            (
                "loudness_db".into(),
                Dynamic::from(analysis.loudness.rms_db as f64),
            ),
            (
                "bass_balance_db".into(),
                Dynamic::from(analysis.spectral.bass_balance_db as f64),
            ),
            (
                "vocal_darkness_db_oct".into(),
                Dynamic::from(analysis.spectral.vocal_darkness_db_oct as f64),
            ),
            (
                "breathiness_db".into(),
                Dynamic::from(analysis.voice.breathiness_db as f64),
            ),
            (
                "voice_presence".into(),
                Dynamic::from(analysis.pitch.voice_presence as f64),
            ),
        ]);

        let target = rhai::Map::from([
            (
                "brightness_hz".into(),
                Dynamic::from(cfg.targets.brightness_hz as f64),
            ),
            (
                "loudness_db".into(),
                Dynamic::from(cfg.targets.loudness_db as f64),
            ),
            (
                "bass_balance_db".into(),
                Dynamic::from(cfg.targets.bass_balance_db as f64),
            ),
            (
                "vocal_darkness_db_oct".into(),
                Dynamic::from(cfg.targets.vocal_darkness_db_oct as f64),
            ),
        ]);

        let knobs = rhai::Map::from([
            (
                "tilt_high_db".into(),
                Dynamic::from(base.tilt_high_db as f64),
            ),
            ("tilt_low_db".into(), Dynamic::from(base.tilt_low_db as f64)),
            (
                "rms_target_lufs".into(),
                Dynamic::from(base.rms_target_lufs as f64),
            ),
            ("presence_db".into(), Dynamic::from(base.presence_db as f64)),
            ("de_harsh_db".into(), Dynamic::from(base.de_harsh_db as f64)),
            (
                "compression_ratio".into(),
                Dynamic::from(base.compression_ratio as f64),
            ),
        ]);

        let damping = Dynamic::from(cfg.damping as f64);

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
        PathBuf::from("training/acoustic-policy.rhai"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::controller::{
        ControllerConfig, ControllerRanges, ControllerSensitivity, ControllerTargets,
    };
    use crate::dsp::SmoothingOptions;
    use std::io::Write;

    fn test_script() -> String {
        r#"
fn correct(measured, target, knobs, damping) {
    let err = target.brightness_hz - measured.brightness_hz;
    if err < -100.0 {
        knobs.tilt_high_db = knobs.tilt_high_db - 3.0 * damping;
    }
    knobs
}

fn clamp(val, lo, hi) {
    if val < lo { lo } else if val > hi { hi } else { val }
}
"#
        .to_string()
    }

    fn write_temp_script(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::with_suffix(".rhai").unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    fn make_analysis(brightness: f32, loudness: f32) -> AnalysisResult {
        let mut a = AnalysisResult {
            temporal: Default::default(),
            loudness: Default::default(),
            spectral: Default::default(),
            pitch: Default::default(),
            voice: Default::default(),
            f0_contour: vec![],
            energy_envelope: vec![],
        };
        a.spectral.brightness_hz = brightness;
        a.loudness.rms_db = loudness;
        a
    }

    #[test]
    fn load_missing_file_returns_none() {
        assert!(PolicyEngine::load(Path::new("/nonexistent/policy.rhai")).is_none());
    }

    #[test]
    fn load_valid_script_returns_some() {
        let f = write_temp_script(&test_script());
        let engine = PolicyEngine::load(f.path());
        assert!(engine.is_some());
    }

    #[test]
    fn load_invalid_script_returns_none() {
        let f = write_temp_script("fn broken( { }");
        let engine = PolicyEngine::load(f.path());
        assert!(engine.is_none());
    }

    #[test]
    fn evaluate_applies_brightness_correction() {
        let f = write_temp_script(&test_script());
        let engine = PolicyEngine::load(f.path()).unwrap();
        let cfg = ControllerConfig::default();
        let base = SmoothingOptions::default();
        let analysis = make_analysis(3400.0, -13.5);

        let result = engine.evaluate(&analysis, &base, &cfg);
        assert!(result.is_some(), "evaluate returned None");
        let (opts, snap) = result.unwrap();
        assert!(
            opts.tilt_high_db < base.tilt_high_db,
            "script should cut treble for bright signal: got {}",
            opts.tilt_high_db
        );
        assert!(snap.enabled);
    }

    #[test]
    fn evaluate_no_correction_when_on_target() {
        let f = write_temp_script(&test_script());
        let engine = PolicyEngine::load(f.path()).unwrap();
        let cfg = ControllerConfig::default();
        let base = SmoothingOptions::default();
        let analysis = make_analysis(2288.0, -13.5);

        let (opts, _) = engine.evaluate(&analysis, &base, &cfg).unwrap();
        assert!(
            (opts.tilt_high_db - base.tilt_high_db).abs() < 0.01,
            "on-target should not change tilt: delta={}",
            opts.tilt_high_db - base.tilt_high_db
        );
    }

    #[test]
    fn reload_picks_up_changed_script() {
        let f = write_temp_script(&test_script());
        let mut engine = PolicyEngine::load(f.path()).unwrap();

        // Overwrite with a different script
        std::fs::write(
            f.path(),
            r#"
fn correct(measured, target, knobs, damping) {
    knobs.tilt_high_db = -99.0;
    knobs
}
fn clamp(val, lo, hi) { val }
"#,
        )
        .unwrap();

        assert!(engine.reload());
        let cfg = ControllerConfig::default();
        let base = SmoothingOptions::default();
        let analysis = make_analysis(3400.0, -13.5);
        let (opts, _) = engine.evaluate(&analysis, &base, &cfg).unwrap();
        assert!(
            (opts.tilt_high_db - (-99.0)).abs() < 0.01,
            "reload should use new script: got {}",
            opts.tilt_high_db
        );
    }
}
