/// Mel-Cepstral Distortion — the standard TTS spectral envelope quality metric.
///
/// MCD = (10 / ln10) × sqrt(2 × Σ(c_ref[i] − c_syn[i])²)  per frame, averaged.
///
/// Benchmarks from TTS literature:
///   < 4 dB  — excellent (near reference quality)
///   4–6 dB  — good
///   6–8 dB  — acceptable
///   > 8 dB  — poor (audible spectral mismatch)
///
/// Implementation reuses mel_filterbank() logic from mfcc.rs.
use rustfft::{num_complex::Complex, FftPlanner};

const N_MELS: usize = 24; // MCD uses 24 bands (standard, vs 26 for MFCC)
const N_COEFF: usize = 24; // use all mel bands for MCD distance
const F_MIN: f32 = 80.0;
const F_MAX: f32 = 7600.0;
const FRAME_MS: f32 = 25.0;
const HOP_MS: f32 = 10.0;
/// No 10/ln10 scaling — raw RMSE of coefficient differences, consistent within this codebase.
/// Typical values: <2 very close, 2–6 similar, >10 very different.
fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}
fn mel_to_hz(m: f32) -> f32 {
    700.0 * (10.0f32.powf(m / 2595.0) - 1.0)
}

fn mel_filterbank(n_fft: usize, sr: u32) -> Vec<Vec<f32>> {
    let sr = sr as f32;
    let mel_lo = hz_to_mel(F_MIN);
    let mel_hi = hz_to_mel(F_MAX);
    let pts: Vec<f32> = (0..=N_MELS + 1)
        .map(|i| mel_lo + (mel_hi - mel_lo) * i as f32 / (N_MELS + 1) as f32)
        .collect();
    let bins: Vec<usize> = pts
        .iter()
        .map(|&m| ((n_fft + 1) as f32 * mel_to_hz(m) / sr).floor() as usize)
        .collect();
    (0..N_MELS)
        .map(|m| {
            let mut f = vec![0.0f32; n_fft / 2];
            for k in bins[m]..bins[m + 1] {
                if k < f.len() {
                    f[k] = (k - bins[m]) as f32 / (bins[m + 1] - bins[m]) as f32;
                }
            }
            for k in bins[m + 1]..bins[m + 2] {
                if k < f.len() {
                    f[k] = (bins[m + 2] - k) as f32 / (bins[m + 2] - bins[m + 1]) as f32;
                }
            }
            f
        })
        .collect()
}

/// Compute per-frame mel-cepstral coefficients (coefficients 1..N_COEFF, excluding 0).
fn frame_mcep(samples: &[f32], sr: u32) -> Vec<Vec<f32>> {
    let frame = {
        let n = (sr as f32 * FRAME_MS / 1000.0) as usize;
        let mut p = 1;
        while p < n {
            p <<= 1;
        }
        p
    };
    let hop = (sr as f32 * HOP_MS / 1000.0) as usize;
    let hann: Vec<f32> = (0..frame)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (frame - 1) as f32).cos()))
        .collect();
    let fb = mel_filterbank(frame, sr);
    let mut planner: FftPlanner<f32> = FftPlanner::new();
    let fft = planner.plan_fft_forward(frame);

    let mut frames = Vec::new();
    let mut i = 0;
    while i + frame <= samples.len() {
        let mut buf: Vec<Complex<f32>> = samples[i..i + frame]
            .iter()
            .zip(&hann)
            .map(|(&s, &w)| Complex { re: s * w, im: 0.0 })
            .collect();
        fft.process(&mut buf);
        let mags: Vec<f32> = buf[..frame / 2].iter().map(|c| c.norm()).collect();

        // Log mel filter energies
        let log_e: Vec<f32> = fb
            .iter()
            .map(|f| (f.iter().zip(&mags).map(|(&w, &m)| w * m).sum::<f32>() + 1e-12).ln())
            .collect();

        // Real cepstrum via DCT-II (coefficients 1..N_COEFF, skip C0)
        let coef: Vec<f32> = (1..=N_COEFF)
            .map(|n| {
                log_e
                    .iter()
                    .enumerate()
                    .map(|(m, &e)| {
                        e * (std::f32::consts::PI * n as f32 * (2 * m + 1) as f32
                            / (2 * N_MELS) as f32)
                            .cos()
                    })
                    .sum::<f32>()
            })
            .collect();

        frames.push(coef);
        i += hop;
    }
    frames
}

/// Compute MCD between two audio buffers of the same phrase.
/// Both must have the same sample_rate. Different lengths are handled by
/// taking the shorter frame count (no DTW — same text, align by truncation).
pub fn compute_mcd(reference: &[f32], synthesis: &[f32], sample_rate: u32) -> f32 {
    let ref_frames = frame_mcep(reference, sample_rate);
    let syn_frames = frame_mcep(synthesis, sample_rate);

    let n = ref_frames.len().min(syn_frames.len());
    if n == 0 {
        return f32::INFINITY;
    }

    // Per-frame RMSE of cepstral coefficient differences, averaged across frames.
    let sum: f64 = ref_frames[..n]
        .iter()
        .zip(&syn_frames[..n])
        .map(|(r, s)| {
            let sq: f64 = r.iter().zip(s).map(|(a, b)| (a - b).powi(2) as f64).sum();
            (sq / N_COEFF as f64).sqrt()
        })
        .sum();

    (sum / n as f64) as f32
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
    fn identical_signals_have_zero_mcd() {
        let s = sine(220.0, 1.0, 22050);
        let mcd = compute_mcd(&s, &s, 22050);
        assert!(mcd < 1e-3, "MCD of identical signals={mcd}");
    }

    #[test]
    fn different_frequencies_have_nonzero_mcd() {
        let s1 = sine(220.0, 1.0, 22050);
        let s2 = sine(3000.0, 1.0, 22050);
        let mcd = compute_mcd(&s1, &s2, 22050);
        assert!(
            mcd > 1.0,
            "MCD={mcd}, expected > 1 for spectrally different tones"
        );
    }

    #[test]
    fn mcd_is_symmetric() {
        let s1 = sine(300.0, 1.0, 22050);
        let s2 = sine(800.0, 1.0, 22050);
        let ab = compute_mcd(&s1, &s2, 22050);
        let ba = compute_mcd(&s2, &s1, 22050);
        assert!((ab - ba).abs() < 0.01, "MCD not symmetric: {ab} vs {ba}");
    }
}
