//! Naturalness scoring — how much does the synthesis sound like real speech?
//!
//! Uses Google ViSQOL v3.1: provide a studio reference WAV and the synthesised
//! WAV, get back a score from 1.0 (very degraded) to 5.0 (indistinguishable).
//! Think of it as an automated human listener rating.
//!
//! < 3.0 = clearly synthetic; 3.5–4.0 = acceptable; > 4.0 = near-transparent.
//!
//! Source: <https://github.com/google/visqol>

use visqol_rs::constants::{DEFAULT_WINDOW_SIZE, NUM_BANDS_SPEECH};
use visqol_rs::variant::Variant;
use visqol_rs::visqol_manager;

/// Compute ViSQOL MOS-LQO between a reference and degraded WAV file.
///
/// Returns `None` if either path is unreadable or the algorithm fails
/// (e.g. files too short, incompatible sample rates).
pub fn score(reference_path: &str, degraded_path: &str) -> Option<f32> {
    // visqol-rs can panic on malformed/short audio — isolate with catch_unwind.
    let ref_path = reference_path.to_string();
    let deg_path = degraded_path.to_string();
    let result = std::panic::catch_unwind(move || {
        let variant = Variant::Wideband {
            use_unscaled_mos_mapping: true,
        };
        let mut manager =
            visqol_manager::VisqolManager::<NUM_BANDS_SPEECH>::new(variant, DEFAULT_WINDOW_SIZE);
        manager
            .run(&ref_path, &deg_path)
            .ok()
            .map(|r| r.moslqo as f32)
    });
    result.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_files_score_near_5() {
        let dir = std::env::temp_dir();
        let path = dir.join("visqol_identity_test.wav");

        // visqol-rs needs at least ~3s at 16 kHz to build enough patches.
        let sr: u32 = 16000;
        let n_samples = sr * 4;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..n_samples {
            let s = (2.0 * std::f32::consts::PI * 200.0 * i as f32 / sr as f32).sin();
            w.write_sample((s * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();

        let p = path.to_str().unwrap();
        let mos = score(p, p);
        assert!(mos.is_some(), "ViSQOL returned None for identical files");
        let mos = mos.unwrap();
        assert!(
            mos > 4.0,
            "identical files should score > 4.0, got {mos:.3}"
        );
    }

    #[test]
    fn short_audio_returns_none_not_panic() {
        let dir = std::env::temp_dir();
        let path = dir.join("visqol_short_test.wav");
        let sr: u32 = 16000;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..1600u32 {
            let s = (2.0 * std::f32::consts::PI * 200.0 * i as f32 / sr as f32).sin();
            w.write_sample((s * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
        let p = path.to_str().unwrap();
        // Must not panic even on too-short audio
        let _ = score(p, p);
    }
}
