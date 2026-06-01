//! ECAPA-TDNN speaker embedding via ONNX Runtime.
//!
//! The model file is exported once via `just export-ecapa` and stored at
//! `rvc/models/pretrained/ecapa-voxceleb.onnx`.
//!
//! Input:  raw waveform `[1, T]` float32 at 16 kHz mono
//! Output: speaker embedding `[1, 192]` (L2-normalised before cosine comparison)
//!
//! If the ONNX file is absent, all functions return `None` gracefully so the
//! rest of the pipeline continues with MFCC-based similarity instead.

use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;

/// 192-dimensional ECAPA-TDNN speaker embedding.
pub type EcapaEmbedding = [f32; 192];

/// Compute cosine similarity between two L2-normalised embeddings. Returns [−1, 1].
pub fn cosine_sim(a: &EcapaEmbedding, b: &EcapaEmbedding) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a * mag_b < 1e-10 {
        return 0.0;
    }
    (dot / (mag_a * mag_b)).clamp(-1.0, 1.0)
}

/// Load ECAPA-TDNN ONNX session from disk. Returns `None` if file is absent.
pub fn load_session(onnx_path: &Path) -> Option<Session> {
    if !onnx_path.exists() {
        return None;
    }
    Session::builder().ok()?.commit_from_file(onnx_path).ok()
}

/// Extract a speaker embedding from 16 kHz mono samples.
/// Returns `None` if inference fails or input is too short (<100 ms).
pub fn extract(session: &mut Session, samples_16k: &[f32]) -> Option<EcapaEmbedding> {
    let t = samples_16k.len();
    if t < 1600 {
        return None;
    }

    let input = Tensor::<f32>::from_array(([1usize, t], samples_16k.to_vec())).ok()?;
    let outputs = session
        .run(ort::inputs!["wav" => input])
        .map_err(|e| tracing::warn!("ecapa inference failed: {e}"))
        .ok()?;

    let (_shape, emb_slice) = outputs["embedding"].try_extract_tensor::<f32>().ok()?;
    if emb_slice.len() != 192 {
        return None;
    }

    let norm: f32 = emb_slice.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mut out = [0f32; 192];
    if norm > 1e-8 {
        for (o, &v) in out.iter_mut().zip(emb_slice) {
            *o = v / norm;
        }
    }
    Some(out)
}

/// Resample to 16 kHz via linear interpolation.
pub fn to_16k(samples: &[f32], src_rate: u32) -> Vec<f32> {
    if src_rate == 16000 {
        return samples.to_vec();
    }
    let ratio = src_rate as f64 / 16000.0;
    let out_len = (samples.len() as f64 / ratio).ceil() as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(samples.len() - 1);
            let frac = (pos - lo as f64) as f32;
            samples[lo] * (1.0 - frac) + samples[hi] * frac
        })
        .collect()
}

/// Compute a corpus-mean embedding from a set of (samples, sample_rate) pairs.
/// The result is L2-normalised and can be used directly with `cosine_sim`.
pub fn corpus_mean(session: &mut Session, all_samples: &[(&[f32], u32)]) -> Option<EcapaEmbedding> {
    let mut embeds: Vec<EcapaEmbedding> = Vec::new();
    for (s, sr) in all_samples {
        let s16 = to_16k(s, *sr);
        if let Some(emb) = extract(session, &s16) {
            embeds.push(emb);
        }
    }

    if embeds.is_empty() {
        return None;
    }

    let mut mean = [0f32; 192];
    for emb in &embeds {
        for (m, &v) in mean.iter_mut().zip(emb) {
            *m += v;
        }
    }
    let n = embeds.len() as f32;
    for m in mean.iter_mut() {
        *m /= n;
    }
    let norm: f32 = mean.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-8 {
        for m in mean.iter_mut() {
            *m /= norm;
        }
    }
    Some(mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_sim_identical() {
        let v = [0.5f32; 192];
        assert!((cosine_sim(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_sim_orthogonal() {
        let mut a = [0f32; 192];
        let mut b = [0f32; 192];
        a[0] = 1.0;
        b[1] = 1.0;
        assert!(cosine_sim(&a, &b).abs() < 1e-5);
    }

    #[test]
    fn to_16k_passthrough() {
        let s: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = to_16k(&s, 16000);
        assert_eq!(out.len(), 100);
    }

    #[test]
    fn to_16k_downsamples() {
        let s = vec![0.0f32; 4800];
        let out = to_16k(&s, 48000);
        assert_eq!(out.len(), 1600);
    }

    #[test]
    fn load_session_absent_returns_none() {
        assert!(load_session(Path::new("/nonexistent/ecapa.onnx")).is_none());
    }
}
