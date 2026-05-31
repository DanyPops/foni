/// Speaker similarity via MFCC cosine distance.
///
/// A speaker-embedding model (e.g. Resemblyzer d-vectors) would be ideal, but
/// requires Python 3.14-incompatible webrtcvad. This pure-Rust approximation
/// computes the cosine similarity between frame-averaged MFCC vectors — a
/// reasonable proxy for same-speaker verification without a neural model.
///
/// Reference: store a pre-computed MFCC vector for the studio WAV as a JSON
/// fixture (`baseline/stalker/speaker/<name>-mfcc.json`). At test time, compute
/// the synthesis MFCC and compare via cosine similarity.
///
/// Threshold: > 0.85 = same speaker, 0.70–0.85 = similar, < 0.70 = different.
use crate::mfcc::compute_mfcc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerEmbedding {
    pub _source: String,
    pub coefficients: Vec<f32>,
}

/// Compute a speaker embedding from a WAV sample buffer.
pub fn embed(samples: &[f32], sample_rate: u32, label: &str) -> SpeakerEmbedding {
    SpeakerEmbedding {
        _source: label.to_string(),
        coefficients: compute_mfcc(samples, sample_rate),
    }
}

/// Cosine similarity between two embedding vectors. Returns value in [−1, 1].
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "embeddings must have same dimension");
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a * mag_b < 1e-10 {
        return 0.0;
    }
    (dot / (mag_a * mag_b)).clamp(-1.0, 1.0)
}

/// Compare synthesis against a stored reference embedding.
pub fn speaker_similarity(reference: &SpeakerEmbedding, synthesis: &SpeakerEmbedding) -> f32 {
    cosine_similarity(&reference.coefficients, &synthesis.coefficients)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(freq: f32, n: usize, sr: u32) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sr as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn same_signal_max_similarity() {
        let s = sine(220.0, 22050, 22050);
        let a = embed(&s, 22050, "a");
        let b = embed(&s, 22050, "b");
        let sim = speaker_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-5, "sim={sim}");
    }

    #[test]
    fn different_signals_lower_similarity() {
        let s1 = sine(220.0, 22050, 22050);
        let s2 = sine(3000.0, 22050, 22050);
        let a = embed(&s1, 22050, "low");
        let b = embed(&s2, 22050, "high");
        let sim = speaker_similarity(&a, &b);
        assert!(sim < 0.9, "sim={sim}");
    }
}
