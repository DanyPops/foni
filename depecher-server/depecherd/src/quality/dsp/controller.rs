//! Reactive DSP controller — auto-corrects voice quality per utterance.
//!
//! Measures the raw RVC output, compares against the Sidorovich studio target,
//! and computes corrected DSP knob settings via a proportional controller.
//!
//! All configuration (targets, sensitivity, damping, ranges) loaded from
//! `rvc/dsp-defaults.json` at startup — no recompilation needed to tune.

use crate::quality::dsp::SmoothingOptions;
use depecher_analyse::AnalysisResult;
use serde::{Deserialize, Serialize};

/// Loaded from `dsp-defaults.json` at startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerConfig {
    pub enabled: bool,
    #[serde(default = "default_damping")]
    pub damping: f32,
    pub targets: ControllerTargets,
    pub sensitivity: ControllerSensitivity,
    pub ranges: ControllerRanges,
}

fn default_damping() -> f32 {
    0.6
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerTargets {
    pub brightness_hz: f32,
    pub loudness_db: f32,
    pub bass_balance_db: f32,
    pub vocal_darkness_db_oct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerSensitivity {
    pub brightness_per_tilt_high: f32,
    #[serde(default = "default_brightness_per_de_harsh")]
    pub brightness_per_de_harsh: f32,
    pub loudness_per_rms_lufs: f32,
    pub bass_balance_per_tilt_low: f32,
    pub bass_balance_per_presence: f32,
    pub darkness_per_tilt_high: f32,
}

fn default_brightness_per_de_harsh() -> f32 {
    -80.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerRanges {
    pub tilt_low_db: [f32; 2],
    pub tilt_high_db: [f32; 2],
    pub rms_target_lufs: [f32; 2],
    pub presence_db: [f32; 2],
    #[serde(default = "default_de_harsh_range")]
    pub de_harsh_db: [f32; 2],
}

fn default_de_harsh_range() -> [f32; 2] {
    [-12.0, 0.0]
}

/// What the controller decided — for the inspection endpoint.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ControllerSnapshot {
    pub enabled: bool,
    pub measured_brightness_hz: f32,
    pub measured_loudness_db: f32,
    pub measured_bass_balance_db: f32,
    pub measured_vocal_darkness: f32,
    pub target_brightness_hz: f32,
    pub target_loudness_db: f32,
    pub target_bass_balance_db: f32,
    pub target_vocal_darkness: f32,
    pub correction_tilt_low_db: f32,
    pub correction_tilt_high_db: f32,
    pub correction_rms_lufs: f32,
    pub correction_presence_db: f32,
    pub correction_de_harsh_db: f32,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            damping: 0.9,
            targets: ControllerTargets {
                brightness_hz: 2288.0,
                loudness_db: -13.5,
                bass_balance_db: 2.8,
                vocal_darkness_db_oct: -6.4,
            },
            sensitivity: ControllerSensitivity {
                brightness_per_tilt_high: 92.0,
                brightness_per_de_harsh: 0.16,
                loudness_per_rms_lufs: 0.975,
                bass_balance_per_tilt_low: 0.21,
                bass_balance_per_presence: -0.36,
                darkness_per_tilt_high: 0.073,
            },
            ranges: ControllerRanges {
                tilt_low_db: [-4.0, 16.0],
                tilt_high_db: [-24.0, 0.0],
                rms_target_lufs: [-22.0, -4.0],
                presence_db: [-6.0, 6.0],
                de_harsh_db: [-12.0, 0.0],
            },
        }
    }
}

/// Compute corrected DSP options based on measured vs target acoustic metrics.
pub fn correct(
    analysis: &AnalysisResult,
    base: &SmoothingOptions,
    cfg: &ControllerConfig,
) -> (SmoothingOptions, ControllerSnapshot) {
    let t = &cfg.targets;
    let s = &cfg.sensitivity;
    let r = &cfg.ranges;
    let d = cfg.damping;

    let m_bright = analysis.spectral.brightness_hz;
    let m_loud = analysis.loudness.rms_db;
    let m_bass = analysis.spectral.bass_balance_db;
    let m_dark = analysis.spectral.vocal_darkness_db_oct;

    let err_bright = t.brightness_hz - m_bright;
    let err_loud = t.loudness_db - m_loud;
    let err_bass = t.bass_balance_db - m_bass;
    let err_dark = t.vocal_darkness_db_oct - m_dark;

    let delta_rms = err_loud / s.loudness_per_rms_lufs * d;
    let delta_tilt_low = err_bass / s.bass_balance_per_tilt_low * d;
    let delta_presence = err_bass / s.bass_balance_per_presence * d;
    // Brightness: tilt shelf is the main lever, de-harsh is negligible per calibration
    let delta_tilt_high_bright = err_bright / s.brightness_per_tilt_high;
    let delta_tilt_high_dark = err_dark / s.darkness_per_tilt_high;
    let delta_tilt_high = (delta_tilt_high_bright * 0.7 + delta_tilt_high_dark * 0.3) * d;
    // De-harsh at 3.5kHz has minimal centroid effect (0.16 Hz/dB) — apply a fixed
    // cut proportional to brightness error instead of dividing by near-zero sensitivity
    let delta_de_harsh = if err_bright < -200.0 {
        -4.0 * d
    } else if err_bright < -100.0 {
        -2.0 * d
    } else {
        0.0
    };

    let mut opts = base.clone();
    opts.tilt_high_db =
        (base.tilt_high_db + delta_tilt_high).clamp(r.tilt_high_db[0], r.tilt_high_db[1]);
    opts.rms_target_lufs =
        (base.rms_target_lufs + delta_rms).clamp(r.rms_target_lufs[0], r.rms_target_lufs[1]);
    opts.tilt_low_db =
        (base.tilt_low_db + delta_tilt_low).clamp(r.tilt_low_db[0], r.tilt_low_db[1]);
    opts.presence_db =
        (base.presence_db + delta_presence * 0.5).clamp(r.presence_db[0], r.presence_db[1]);
    opts.de_harsh_db =
        (base.de_harsh_db + delta_de_harsh).clamp(r.de_harsh_db[0], r.de_harsh_db[1]);

    let snap = ControllerSnapshot {
        enabled: true,
        measured_brightness_hz: m_bright,
        measured_loudness_db: m_loud,
        measured_bass_balance_db: m_bass,
        measured_vocal_darkness: m_dark,
        target_brightness_hz: t.brightness_hz,
        target_loudness_db: t.loudness_db,
        target_bass_balance_db: t.bass_balance_db,
        target_vocal_darkness: t.vocal_darkness_db_oct,
        correction_tilt_low_db: opts.tilt_low_db - base.tilt_low_db,
        correction_tilt_high_db: opts.tilt_high_db - base.tilt_high_db,
        correction_rms_lufs: opts.rms_target_lufs - base.rms_target_lufs,
        correction_presence_db: opts.presence_db - base.presence_db,
        correction_de_harsh_db: opts.de_harsh_db - base.de_harsh_db,
    };

    (opts, snap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_analysis(brightness: f32, loudness: f32, bass: f32, darkness: f32) -> AnalysisResult {
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
        a.spectral.bass_balance_db = bass;
        a.spectral.vocal_darkness_db_oct = darkness;
        a
    }

    #[test]
    fn too_bright_cuts_treble() {
        let a = make_analysis(3400.0, -13.5, 2.8, -6.4);
        let base = SmoothingOptions::default();
        let cfg = ControllerConfig::default();
        let (opts, snap) = correct(&a, &base, &cfg);
        assert!(opts.tilt_high_db < base.tilt_high_db, "should cut treble");
        assert!(snap.correction_tilt_high_db < 0.0);
    }

    #[test]
    fn too_quiet_raises_loudness() {
        let a = make_analysis(2288.0, -19.0, 2.8, -6.4);
        let base = SmoothingOptions::default();
        let cfg = ControllerConfig::default();
        let (opts, _) = correct(&a, &base, &cfg);
        assert!(
            opts.rms_target_lufs > base.rms_target_lufs,
            "should raise RMS: corrected={}, base={}",
            opts.rms_target_lufs,
            base.rms_target_lufs
        );
    }

    #[test]
    fn on_target_makes_small_corrections() {
        let a = make_analysis(2288.0, -13.5, 2.8, -6.4);
        let base = SmoothingOptions::default();
        let cfg = ControllerConfig::default();
        let (_, snap) = correct(&a, &base, &cfg);
        assert!(snap.correction_tilt_high_db.abs() < 0.5);
        assert!(snap.correction_rms_lufs.abs() < 0.5);
    }

    #[test]
    fn corrections_stay_in_safe_range() {
        let a = make_analysis(8000.0, -30.0, -10.0, 0.0);
        let base = SmoothingOptions::default();
        let cfg = ControllerConfig::default();
        let (opts, _) = correct(&a, &base, &cfg);
        assert!(opts.tilt_high_db >= cfg.ranges.tilt_high_db[0]);
        assert!(opts.tilt_high_db <= cfg.ranges.tilt_high_db[1]);
        assert!(opts.rms_target_lufs >= cfg.ranges.rms_target_lufs[0]);
        assert!(opts.rms_target_lufs <= cfg.ranges.rms_target_lufs[1]);
    }
}
