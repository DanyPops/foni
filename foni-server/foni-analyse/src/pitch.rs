use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PitchMetrics {
    /// Mean F0 across voiced frames in Hz.
    pub f0_mean_hz:     f32,
    /// Standard deviation of F0 in Hz — pitch variation.
    pub f0_stddev_hz:   f32,
    /// F0 slope in semitones/second — intonation direction.
    pub f0_slope_semi:  f32,
    /// Fraction of frames classified as voiced.
    pub voiced_ratio:   f32,
}

pub fn compute(samples: &[f32], sample_rate: u32) -> PitchMetrics {
    // TODO (FON-TSK-13): replace stub with pyin crate integration.
    // pyin::pyin(samples, sample_rate, fmin=80.0, fmax=500.0) → Vec<Option<f32>>
    // Each Some(f0) = voiced frame, None = unvoiced.
    // For now return zeroed metrics so the crate compiles and tests run.
    let _ = (samples, sample_rate);
    PitchMetrics {
        f0_mean_hz:    0.0,
        f0_stddev_hz:  0.0,
        f0_slope_semi: 0.0,
        voiced_ratio:  0.0,
    }
}
