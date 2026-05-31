pub mod gap;
pub mod loudness;
pub mod mfcc;
pub mod pitch;
pub mod report;
pub mod spectral;
pub mod temporal;
pub mod voice;
pub mod wav;

pub use gap::{compute_gap, GapResult, GapRow, TargetTensor, Verdict};
pub use loudness::LoudnessMetrics;
pub use mfcc::mfcc_distance;
pub use pitch::PitchMetrics;
pub use report::{format_gap_summary, format_gap_table};
pub use spectral::SpectralMetrics;
pub use temporal::TemporalMetrics;
pub use voice::VoiceMetrics;
pub use wav::decode_wav;

use serde::Serialize;

/// Full analysis result for one audio buffer.
/// Serialised to JSON by the /analyse HTTP endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisResult {
    pub temporal:  TemporalMetrics,
    pub spectral:  SpectralMetrics,
    pub loudness:  LoudnessMetrics,
    pub pitch:     PitchMetrics,
    pub voice:     VoiceMetrics,
}

/// Run the full analysis pipeline on raw f32 samples.
pub fn analyse(samples: &[f32], sample_rate: u32) -> AnalysisResult {
    AnalysisResult {
        temporal: temporal::compute(samples, sample_rate),
        spectral: spectral::compute(samples, sample_rate),
        loudness: loudness::compute(samples, sample_rate),
        pitch:    pitch::compute(samples, sample_rate),
        voice:    voice::compute(samples, sample_rate),
    }
}
