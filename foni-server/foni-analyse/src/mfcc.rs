use rustfft::{FftPlanner, num_complex::Complex};

const N_MELS:  usize = 26;
const N_MFCCS: usize = 13;
const F_MIN:   f32   = 80.0;
const F_MAX:   f32   = 8000.0;
const FRAME_MS: f32  = 25.0;
const HOP_MS:   f32  = 10.0;

fn hz_to_mel(hz: f32) -> f32 { 2595.0 * (1.0 + hz / 700.0).log10() }
fn mel_to_hz(mel: f32) -> f32 { 700.0 * (10.0f32.powf(mel / 2595.0) - 1.0) }

fn mel_filterbank(n_fft: usize, sample_rate: u32) -> Vec<Vec<f32>> {
    let sr = sample_rate as f32;
    let mel_min = hz_to_mel(F_MIN);
    let mel_max = hz_to_mel(F_MAX);
    let mel_points: Vec<f32> = (0..=N_MELS + 1)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (N_MELS + 1) as f32)
        .collect();
    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();
    let bins: Vec<usize> = hz_points
        .iter()
        .map(|&f| ((n_fft as f32 + 1.0) * f / sr).floor() as usize)
        .collect();

    (0..N_MELS)
        .map(|m| {
            let mut filter = vec![0.0f32; n_fft / 2];
            for k in bins[m]..bins[m + 1] {
                if k < filter.len() {
                    filter[k] = (k - bins[m]) as f32 / (bins[m + 1] - bins[m]) as f32;
                }
            }
            for k in bins[m + 1]..bins[m + 2] {
                if k < filter.len() {
                    filter[k] = (bins[m + 2] - k) as f32 / (bins[m + 2] - bins[m + 1]) as f32;
                }
            }
            filter
        })
        .collect()
}

/// Compute frame-averaged MFCC vector (N_MFCCS coefficients).
pub fn compute_mfcc(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let sr = sample_rate as f32;
    let frame_size = {
        let n = (sr * FRAME_MS / 1000.0) as usize;
        let mut p = 1;
        while p < n { p <<= 1; }
        p
    };
    let hop_size = (sr * HOP_MS / 1000.0) as usize;

    let mut planner: FftPlanner<f32> = FftPlanner::new();
    let fft = planner.plan_fft_forward(frame_size);
    let hann: Vec<f32> = (0..frame_size)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (frame_size - 1) as f32).cos()))
        .collect();
    let filterbank = mel_filterbank(frame_size, sample_rate);

    let mut sum = vec![0.0f64; N_MFCCS];
    let mut frame_count = 0usize;

    let mut i = 0;
    while i + frame_size <= samples.len() {
        let mut buf: Vec<Complex<f32>> = samples[i..i + frame_size]
            .iter()
            .zip(&hann)
            .map(|(&s, &w)| Complex { re: s * w, im: 0.0 })
            .collect();
        fft.process(&mut buf);

        let mags: Vec<f32> = buf[..frame_size / 2].iter().map(|c| c.norm()).collect();
        let log_energies: Vec<f32> = filterbank
            .iter()
            .map(|f| {
                let e: f32 = f.iter().zip(&mags).map(|(&w, &m)| w * m).sum();
                (e + 1e-10).ln()
            })
            .collect();

        // DCT-II: N_MFCCS coefficients
        for n in 0..N_MFCCS {
            let c: f32 = log_energies
                .iter()
                .enumerate()
                .map(|(m, &e)| {
                    e * (std::f32::consts::PI * n as f32 * (2 * m + 1) as f32 / (2 * N_MELS) as f32).cos()
                })
                .sum();
            sum[n] += c as f64;
        }
        frame_count += 1;
        i += hop_size;
    }

    if frame_count == 0 {
        return vec![0.0; N_MFCCS];
    }
    sum.iter().map(|&s| (s / frame_count as f64) as f32).collect()
}

/// Mean absolute difference between two MFCC vectors — timbre distance.
/// 0.0 = identical, higher = more different.
pub fn mfcc_distance(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "MFCC vectors must be same length");
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n).map(|i| (2.0 * PI * freq * i as f32 / sr as f32).sin() * 0.5).collect()
    }

    #[test]
    fn same_signal_has_zero_distance() {
        let s = sine(440.0, 1.0, 22050);
        let a = compute_mfcc(&s, 22050);
        let b = compute_mfcc(&s, 22050);
        assert_eq!(mfcc_distance(&a, &b), 0.0);
    }

    #[test]
    fn different_signals_have_nonzero_distance() {
        let s1 = sine(440.0, 1.0, 22050);
        let s2 = sine(2000.0, 1.0, 22050);
        let a = compute_mfcc(&s1, 22050);
        let b = compute_mfcc(&s2, 22050);
        assert!(mfcc_distance(&a, &b) > 1.0, "distance={}", mfcc_distance(&a, &b));
    }

    #[test]
    fn returns_correct_coefficient_count() {
        let s = sine(440.0, 0.5, 22050);
        let m = compute_mfcc(&s, 22050);
        assert_eq!(m.len(), N_MFCCS);
    }
}
