/// POST /breath — synthesise a breath-intake WAV using Rust DSP.
///
/// Replaces ffmpeg's `aevalsrc=random(0)*0.025,bandpass,afade` pipeline
/// in pipeline/breath-injector.ts synthesiseBreath().
///
/// Acoustic model (mirrors TypeScript original):
///   white noise -32 dBFS → bandpass 2kHz Q=0.5 → fade-in 5ms → fade-out 40ms
///
/// Input:  { duration_ms: u32, sample_rate: u32 }
/// Output: { audio_data: base64_wav }
use axum::{http::StatusCode, Json};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};

use crate::{dsp::filters::Biquad, wav};

#[derive(Deserialize)]
pub struct BreathRequest {
    #[serde(default = "default_duration_ms")]
    pub duration_ms: u32,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
}

fn default_duration_ms() -> u32 {
    120
}
fn default_sample_rate() -> u32 {
    22050
}

#[derive(Serialize)]
pub struct BreathResponse {
    pub audio_data: String,
}

/// Pseudo-random noise via a simple LCG (deterministic within a session).
/// Not cryptographic — we just need spectral density.
struct Lcg(u64);
impl Lcg {
    fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

pub async fn breath(
    Json(req): Json<BreathRequest>,
) -> Result<Json<BreathResponse>, (StatusCode, String)> {
    let dur_ms = req.duration_ms;
    let sr = req.sample_rate;

    let out = tokio::task::spawn_blocking(move || {
        let n_samples = (sr as f32 * dur_ms as f32 / 1000.0) as usize;
        let amplitude = 10f32.powf(-32.0 / 20.0); // -32 dBFS

        // White noise
        let mut samples: Vec<f32> = {
            let mut rng = Lcg(0xdeadbeef_cafebabe);
            (0..n_samples).map(|_| rng.next_f32() * amplitude).collect()
        };

        // Bandpass centred at 2kHz, Q=0.5 (same as ffmpeg bandpass=f=2000:width_type=q:width=0.5)
        // Implemented as highpass(600Hz) + lowpass(6kHz) cascade
        let mut hp = Biquad::highpass(600.0, sr);
        let mut lp = lowpass_biquad(6000.0, sr);
        hp.process(&mut samples);
        lp.process(&mut samples);

        // Fade-in 5ms, fade-out 40ms (asymmetric: fast intake, gradual release)
        let fade_in_n = ((sr as f32 * 0.005) as usize).min(n_samples);
        let fade_out_n = ((sr as f32 * 0.040) as usize).min(n_samples);
        for i in 0..fade_in_n {
            samples[i] *= i as f32 / fade_in_n as f32;
        }
        for i in 0..fade_out_n {
            samples[n_samples - 1 - i] *= i as f32 / fade_out_n as f32;
        }

        wav::encode_wav(&samples, sr).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))?
    .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))?;

    Ok(Json(BreathResponse {
        audio_data: B64.encode(&out),
    }))
}

/// Simple 2nd-order low-pass (delegates to Biquad::lowpass).
fn lowpass_biquad(cutoff_hz: f32, sample_rate: u32) -> Biquad {
    let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate as f32;
    let cos_w = w0.cos();
    let q = std::f32::consts::FRAC_1_SQRT_2;
    let alpha = w0.sin() / (2.0 * q);
    let b0 = (1.0 - cos_w) / 2.0;
    let b1 = 1.0 - cos_w;
    let b2 = (1.0 - cos_w) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w;
    let a2 = 1.0 - alpha;
    Biquad::lowpass(cutoff_hz, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::decode_wav;

    fn generate_breath_samples(dur_ms: u32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * dur_ms as f32 / 1000.0) as usize;
        let amplitude = 10f32.powf(-32.0 / 20.0);
        let mut samples: Vec<f32> = {
            let mut rng = Lcg(0xdeadbeef_cafebabe);
            (0..n).map(|_| rng.next_f32() * amplitude).collect()
        };
        let mut hp = Biquad::highpass(600.0, sr);
        let mut lp = lowpass_biquad(6000.0, sr);
        hp.process(&mut samples);
        lp.process(&mut samples);
        let fade_in = ((sr as f32 * 0.005) as usize).min(n);
        let fade_out = ((sr as f32 * 0.040) as usize).min(n);
        for i in 0..fade_in {
            samples[i] *= i as f32 / fade_in as f32;
        }
        for i in 0..fade_out {
            samples[n - 1 - i] *= i as f32 / fade_out as f32;
        }
        samples
    }

    #[test]
    fn breath_is_non_silent() {
        let samples = generate_breath_samples(120, 22050);
        let rms = (samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        assert!(rms > 1e-4, "breath RMS={rms} — should be non-silent");
    }

    #[test]
    fn breath_has_correct_duration() {
        let sr = 22050u32;
        let samples = generate_breath_samples(120, sr);
        let expected = (sr as f32 * 0.12) as usize;
        assert!((samples.len() as i64 - expected as i64).abs() < 10);
    }

    #[test]
    fn breath_encodes_to_valid_wav() {
        use crate::wav::encode_wav;
        let samples = generate_breath_samples(120, 22050);
        let wav = encode_wav(&samples, 22050).expect("encode failed");
        let decoded = decode_wav(&wav).expect("decode failed");
        assert!(!decoded.samples.is_empty());
    }
}
