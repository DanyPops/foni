use pyin::{Framing, PYINExecutor, PadMode};
use serde::Serialize;

const FMIN: f64 = 80.0;
const FMAX: f64 = 500.0;
const FRAME_LENGTH: usize = 2048;

#[derive(Debug, Clone, Serialize, Default)]
pub struct PitchMetrics {
    /// Mean F0 across voiced frames in Hz. 0.0 when no voiced frames detected.
    pub f0_mean_hz: f32,
    /// Standard deviation of F0 in Hz — pitch variation. 0.0 = robotic monotone.
    pub f0_stddev_hz: f32,
    /// F0 slope in semitones/second — intonation direction.
    pub f0_slope_semi: f32,
    /// Fraction of frames classified as voiced.
    pub voiced_ratio: f32,
}

/// Run pyin and return both aggregate metrics and per-frame F0 contour.
/// f0_contour[i] = F0 in Hz for frame i, 0.0 if unvoiced.
pub fn compute_with_contour(samples: &[f32], sample_rate: u32) -> (PitchMetrics, Vec<f32>) {
    if samples.len() < FRAME_LENGTH {
        return (
            PitchMetrics {
                f0_mean_hz: 0.0,
                f0_stddev_hz: 0.0,
                f0_slope_semi: 0.0,
                voiced_ratio: 0.0,
            },
            vec![],
        );
    }
    let wav: Vec<f64> = samples.iter().map(|&s| s as f64).collect();
    let mut executor = PYINExecutor::new(FMIN, FMAX, sample_rate, FRAME_LENGTH, None, None, None);
    let (_ts, f0, voiced_flag, _vp) =
        executor.pyin(&wav, f64::NAN, Framing::Center(PadMode::Constant(0.)));

    // Contour: 0.0 for unvoiced frames, F0 Hz for voiced
    let contour: Vec<f32> = f0
        .iter()
        .zip(voiced_flag.iter())
        .map(|(&f, &v)| if v && f.is_finite() { f as f32 } else { 0.0 })
        .collect();

    let metrics = metrics_from_pyin(&f0, &voiced_flag, sample_rate);
    (metrics, contour)
}

pub fn compute(samples: &[f32], sample_rate: u32) -> PitchMetrics {
    compute_with_contour(samples, sample_rate).0
}

fn metrics_from_pyin(f0: &[f64], voiced_flag: &[bool], sample_rate: u32) -> PitchMetrics {
    let total_frames = voiced_flag.len();
    if total_frames == 0 {
        return PitchMetrics {
            f0_mean_hz: 0.0,
            f0_stddev_hz: 0.0,
            f0_slope_semi: 0.0,
            voiced_ratio: 0.0,
        };
    }
    let voiced: Vec<f64> = f0
        .iter()
        .zip(voiced_flag.iter())
        .filter(|(_, &v)| v)
        .map(|(&f, _)| f)
        .filter(|f| f.is_finite())
        .collect();
    let voiced_ratio = voiced_flag.iter().filter(|&&v| v).count() as f32 / total_frames as f32;

    if voiced.is_empty() {
        return PitchMetrics {
            f0_mean_hz: 0.0,
            f0_stddev_hz: 0.0,
            f0_slope_semi: 0.0,
            voiced_ratio,
        };
    }

    let mean = voiced.iter().sum::<f64>() / voiced.len() as f64;
    let variance = voiced.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / voiced.len() as f64;
    let stddev = variance.sqrt();

    // F0 slope: linear regression over voiced frames → semitones/second
    let hop_secs = FRAME_LENGTH as f64 / 4.0 / sample_rate as f64; // default hop = frame/4
    let n = voiced.len() as f64;
    let xs: Vec<f64> = (0..voiced.len()).map(|i| i as f64 * hop_secs).collect();
    let x_mean = xs.iter().sum::<f64>() / n;
    let num: f64 = xs
        .iter()
        .zip(&voiced)
        .map(|(x, f)| (x - x_mean) * (f - mean))
        .sum();
    let den: f64 = xs.iter().map(|x| (x - x_mean).powi(2)).sum();
    let slope_hz_per_sec = if den > 0.0 { num / den } else { 0.0 };
    // Convert Hz/s slope to semitones/s at the mean frequency
    let slope_semi = if mean > 0.0 {
        slope_hz_per_sec / (mean * 2f64.ln() / 12.0)
    } else {
        0.0
    };

    PitchMetrics {
        f0_mean_hz: mean as f32,
        f0_stddev_hz: stddev as f32,
        f0_slope_semi: slope_semi as f32,
        voiced_ratio,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sr as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn pure_sine_f0_near_frequency() {
        let samples = sine(220.0, 1.0, 22050);
        let m = compute(&samples, 22050);
        assert!(m.f0_mean_hz > 0.0, "expected voiced output");
        assert!(
            (m.f0_mean_hz - 220.0).abs() < 30.0,
            "f0_mean_hz={} expected ~220",
            m.f0_mean_hz
        );
    }

    #[test]
    fn pure_sine_high_voiced_ratio() {
        let samples = sine(440.0, 1.0, 22050);
        let m = compute(&samples, 22050);
        assert!(m.voiced_ratio > 0.8, "voiced_ratio={}", m.voiced_ratio);
    }

    #[test]
    fn silence_has_zero_voiced_ratio() {
        let samples = vec![0.0f32; 22050];
        let m = compute(&samples, 22050);
        assert_eq!(m.voiced_ratio, 0.0);
        assert_eq!(m.f0_mean_hz, 0.0);
    }

    #[test]
    fn pure_sine_low_stddev() {
        // Monotone sine = near-zero F0 variation
        let samples = sine(300.0, 2.0, 22050);
        let m = compute(&samples, 22050);
        if m.f0_mean_hz > 0.0 {
            assert!(
                m.f0_stddev_hz < 20.0,
                "stddev={} — expected low for pure tone",
                m.f0_stddev_hz
            );
        }
    }
}

/// Fast F0 estimation via McLeod Pitch Method (MPM).
/// ~50 ms per file vs pyin's ~1400 ms — suitable for batch corpus analysis.
/// Returns (f0_mean_hz, f0_stddev_hz, voiced_ratio); no per-frame contour.
pub fn fast_f0_stats(samples: &[f32], sample_rate: u32) -> (f32, f32, f32) {
    use pitch_detection::detector::mcleod::McLeodDetector;
    use pitch_detection::detector::PitchDetector;

    const HOP: usize = 512;
    const SIZE: usize = 2048;
    const POWER_THRESHOLD: f32 = 5e-4;
    const CLARITY_THRESHOLD: f32 = 0.6;

    let mut detector = McLeodDetector::<f32>::new(SIZE, SIZE / 2);
    let mut f0s: Vec<f32> = Vec::new();
    let mut total_frames = 0usize;

    for start in (0..samples.len().saturating_sub(SIZE)).step_by(HOP) {
        total_frames += 1;
        let frame = &samples[start..start + SIZE];
        if let Some(pitch) = detector.get_pitch(
            frame,
            sample_rate as usize,
            POWER_THRESHOLD,
            CLARITY_THRESHOLD,
        ) {
            let f = pitch.frequency;
            if f >= FMIN as f32 && f <= FMAX as f32 {
                f0s.push(f);
            }
        }
    }

    if f0s.is_empty() {
        return (0.0, 0.0, 0.0);
    }

    let mean = f0s.iter().sum::<f32>() / f0s.len() as f32;
    let var = f0s.iter().map(|&x| (x - mean).powi(2)).sum::<f32>() / f0s.len() as f32;
    let voiced_ratio = f0s.len() as f32 / total_frames.max(1) as f32;

    (mean, var.sqrt(), voiced_ratio)
}
