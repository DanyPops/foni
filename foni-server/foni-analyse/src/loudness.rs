use serde::Serialize;

const FRAME_MS: f32 = 10.0;

/// Per-frame RMS energy envelope (one value per 10ms frame).
pub fn energy_envelope(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let frame = (sample_rate as f32 * FRAME_MS / 1000.0) as usize;
    if frame == 0 {
        return vec![];
    }
    samples
        .chunks(frame)
        .map(|c| (c.iter().map(|&s| s * s).sum::<f32>() / c.len() as f32).sqrt())
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct LoudnessMetrics {
    /// RMS level in dBFS.
    pub rms_db: f32,
    /// Peak level in dBFS.
    pub peak_db: f32,
    /// Peak-to-RMS ratio in dB — dynamic range.
    pub crest_factor: f32,
    /// EBU R128 integrated loudness in LUFS. None if ebur128 fails.
    pub lufs: Option<f32>,
}

pub fn compute(samples: &[f32], sample_rate: u32) -> LoudnessMetrics {
    let rms = (samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

    let rms_db = if rms > 0.0 {
        20.0 * rms.log10()
    } else {
        f32::NEG_INFINITY
    };
    let peak_db = if peak > 0.0 {
        20.0 * peak.log10()
    } else {
        f32::NEG_INFINITY
    };
    let crest_factor = if rms_db.is_finite() && peak_db.is_finite() {
        peak_db - rms_db
    } else {
        0.0
    };

    let lufs = compute_lufs(samples, sample_rate);

    LoudnessMetrics {
        rms_db,
        peak_db,
        crest_factor,
        lufs,
    }
}

fn compute_lufs(samples: &[f32], sample_rate: u32) -> Option<f32> {
    let mut meter = ebur128::EbuR128::new(1, sample_rate, ebur128::Mode::I).ok()?;
    meter.add_frames_f32(samples).ok()?;
    let lufs = meter.loudness_global().ok()? as f32;
    Some(lufs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(amp: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n)
            .map(|i| amp * (2.0 * PI * 440.0 * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn full_scale_sine_rms_near_minus_3db() {
        let samples = sine(1.0, 1.0, 22050);
        let m = compute(&samples, 22050);
        // RMS of sine with amplitude 1.0 = 1/sqrt(2) ≈ −3.01 dBFS
        assert!((m.rms_db - (-3.01)).abs() < 0.1, "rms_db={}", m.rms_db);
    }

    #[test]
    fn silence_gives_neg_inf_rms() {
        let samples = vec![0.0f32; 22050];
        let m = compute(&samples, 22050);
        assert!(m.rms_db.is_infinite());
    }

    #[test]
    fn crest_factor_positive() {
        let samples = sine(0.5, 1.0, 22050);
        let m = compute(&samples, 22050);
        assert!(m.crest_factor > 0.0);
    }
}
