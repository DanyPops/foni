use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VoiceMetrics {
    /// Harmonics-to-noise ratio in dB — naturalness.
    pub hnr_db:    f32,
    /// Cepstral peak prominence — periodicity strength.
    pub cpp:       f32,
    /// Cycle-to-cycle F0 perturbation — roughness.
    pub jitter:    f32,
    /// Cycle-to-cycle amplitude perturbation — breathiness.
    pub shimmer:   f32,
}

pub fn compute(samples: &[f32], sample_rate: u32) -> VoiceMetrics {
    // TODO (FON-TSK-18): implement HNR, CPP, jitter, shimmer.
    // Placeholder so crate compiles.
    let _ = (samples, sample_rate);
    VoiceMetrics { hnr_db: 0.0, cpp: 0.0, jitter: 0.0, shimmer: 0.0 }
}
