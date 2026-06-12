//! Speech emotion recognition via wav2vec2 ONNX model.
//!
//! Model: audeering/wav2vec2-large-robust-12-ft-emotion-msp-dim
//! Input: raw f32 audio at 16kHz, shape [1, time]
//! Output: [arousal, dominance, valence] in range ~0..1

use std::path::Path;

use ort::session::Session;
use ort::value::Tensor;

const MODEL_SAMPLE_RATE: u32 = 16_000;

/// Continuous emotion dimensions from speech audio.
#[derive(Debug, Clone, Copy)]
pub struct ToneReading {
    /// Passive (0) → active (1)
    pub arousal: f32,
    /// Weak (0) → strong (1)
    pub dominance: f32,
    /// Negative (0) → positive (1)
    pub valence: f32,
}

/// Analyze speech audio and return emotion dimensions.
///
/// `samples` must be mono f32 at `sample_rate` Hz.
/// Internally resampled to 16kHz for the model.
pub fn read_tone(
    session: &mut Session,
    samples: &[f32],
    sample_rate: u32,
) -> Result<ToneReading, String> {
    let resampled = if sample_rate == MODEL_SAMPLE_RATE {
        samples.to_vec()
    } else {
        resample(samples, sample_rate, MODEL_SAMPLE_RATE)
    };

    let t = resampled.len();
    let input =
        Tensor::<f32>::from_array(([1usize, t], resampled)).map_err(|e| format!("tensor: {e}"))?;

    let outputs = session
        .run(ort::inputs!["signal" => input])
        .map_err(|e| format!("ort run: {e}"))?;

    let (_shape, logits) = outputs["logits"]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("extract logits: {e}"))?;

    if logits.len() < 3 {
        return Err(format!("expected 3 logits, got {}", logits.len()));
    }

    Ok(ToneReading {
        arousal: logits[0],
        dominance: logits[1],
        valence: logits[2],
    })
}

pub fn load_session(onnx_path: &Path) -> Option<Session> {
    if !onnx_path.exists() {
        return None;
    }
    Session::builder().ok()?.commit_from_file(onnx_path).ok()
}

fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (samples.len() as f64 * ratio).round() as usize;
    let mut out = vec![0.0f32; new_len];
    for (i, sample) in out.iter_mut().enumerate() {
        let src = i as f64 / ratio;
        let idx = src as usize;
        let frac = (src - idx as f64) as f32;
        let a = samples.get(idx).copied().unwrap_or(0.0);
        let b = samples.get(idx + 1).copied().unwrap_or(a);
        *sample = a + frac * (b - a);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_identity() {
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let out = resample(&input, 16000, 16000);
        assert_eq!(out.len(), input.len());
        for (a, b) in out.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn resample_downsample() {
        let input: Vec<f32> = (0..48000).map(|i| (i as f32 / 48000.0).sin()).collect();
        let out = resample(&input, 48000, 16000);
        assert_eq!(out.len(), 16000);
    }
}
