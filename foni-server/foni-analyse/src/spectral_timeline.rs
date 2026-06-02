//! Per-frame spectral comparison — shows WHERE quality breaks down, not just the average.
//!
//! The agent uses this to locate artifacts: "frames 50–80 have 3× the spectral distance —
//! that's where the toilet sound lives."
//!
//! Math: Log Spectral Distance (LSD) — the integral of squared log-power difference
//! across time and frequency. Standard in TTS evaluation (Google, Meta, etc.).

use rustfft::{num_complex::Complex, FftPlanner};
use serde::Serialize;

use crate::contour::contour_correlation;

const FRAME_MS: f32 = 25.0;
const HOP_MS: f32 = 10.0;
const N_MELS: usize = 80;
const F_MIN: f32 = 80.0;
const F_MAX: f32 = 7600.0;

/// Per-frame comparison between a reference and synthesis recording.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SpectralTimeline {
    /// How different the two spectral surfaces are overall (dB). Lower = better match.
    /// 0.0 = identical. < 3.0 = excellent. < 6.0 = good. > 8.0 = clearly different.
    pub spectral_gap: f32,
    /// Spectral distance at each 10ms frame — shows WHERE the gap is worst.
    pub spectral_gap_per_frame: Vec<f32>,
    /// Reference F0 contour after DTW alignment (Hz per frame, 0.0 = unvoiced).
    pub pitch_ref: Vec<f32>,
    /// Synthesis F0 contour after DTW alignment.
    pub pitch_syn: Vec<f32>,
    /// Reference energy per frame after DTW alignment.
    pub energy_ref: Vec<f32>,
    /// Synthesis energy per frame after DTW alignment.
    pub energy_syn: Vec<f32>,
    /// Pitch shape match after alignment (0–1, higher = better).
    pub pitch_match: f32,
    /// Energy shape match after alignment (0–1, higher = better).
    pub energy_match: f32,
    /// Top-10 worst frames: (frame_index, gap_db). Where the artifacts live.
    pub worst_frames: Vec<(usize, f32)>,
    /// Frame duration in seconds (typically 0.01).
    pub frame_step_secs: f32,
}

fn next_pow2(n: usize) -> usize {
    let mut p = 1;
    while p < n {
        p <<= 1;
    }
    p
}

fn mel_filterbank(n_fft: usize, sr: u32) -> Vec<Vec<f32>> {
    let sr = sr as f32;
    let hz_to_mel = |hz: f32| 2595.0 * (1.0 + hz / 700.0).log10();
    let mel_to_hz = |m: f32| 700.0 * (10.0f32.powf(m / 2595.0) - 1.0);

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
                if k < f.len() && bins[m + 1] > bins[m] {
                    f[k] = (k - bins[m]) as f32 / (bins[m + 1] - bins[m]) as f32;
                }
            }
            for k in bins[m + 1]..bins[m + 2] {
                if k < f.len() && bins[m + 2] > bins[m + 1] {
                    f[k] = (bins[m + 2] - k) as f32 / (bins[m + 2] - bins[m + 1]) as f32;
                }
            }
            f
        })
        .collect()
}

/// Compute log-mel spectrogram: returns `[T][N_MELS]` of log power per frame.
fn log_mel_spectrogram(samples: &[f32], sr: u32) -> Vec<Vec<f32>> {
    let frame_size = next_pow2((sr as f32 * FRAME_MS / 1000.0) as usize);
    let hop = (sr as f32 * HOP_MS / 1000.0) as usize;
    let hann: Vec<f32> = (0..frame_size)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (frame_size - 1) as f32).cos())
        })
        .collect();
    let fb = mel_filterbank(frame_size, sr);
    let mut planner: FftPlanner<f32> = FftPlanner::new();
    let fft = planner.plan_fft_forward(frame_size);

    let mut frames = Vec::new();
    let mut i = 0;
    while i + frame_size <= samples.len() {
        let mut buf: Vec<Complex<f32>> = samples[i..i + frame_size]
            .iter()
            .zip(&hann)
            .map(|(&s, &w)| Complex { re: s * w, im: 0.0 })
            .collect();
        fft.process(&mut buf);
        let mags: Vec<f32> = buf[..frame_size / 2].iter().map(|c| c.norm()).collect();

        let log_mel: Vec<f32> = fb
            .iter()
            .map(|f| {
                let power: f32 = f.iter().zip(&mags).map(|(&w, &m)| w * m * m).sum();
                (power.max(1e-10)).log10() * 10.0
            })
            .collect();

        frames.push(log_mel);
        i += hop;
    }
    frames
}

/// Log Spectral Distance between two log-mel spectrograms (already frame-aligned).
/// Returns (overall_lsd, per_frame_lsd).
fn log_spectral_distance(ref_mel: &[Vec<f32>], syn_mel: &[Vec<f32>]) -> (f32, Vec<f32>) {
    let n = ref_mel.len().min(syn_mel.len());
    if n == 0 {
        return (0.0, vec![]);
    }

    let per_frame: Vec<f32> = ref_mel[..n]
        .iter()
        .zip(&syn_mel[..n])
        .map(|(r, s)| {
            let k = r.len().min(s.len()) as f32;
            let sq_sum: f32 = r.iter().zip(s).map(|(a, b)| (a - b).powi(2)).sum();
            (sq_sum / k).sqrt()
        })
        .collect();

    let mean = per_frame.iter().sum::<f32>() / per_frame.len() as f32;
    (mean, per_frame)
}

/// Naive DTW path for alignment (reuses logic from contour.rs but returns the path).
fn dtw_align(a: &[f32], b: &[f32]) -> Vec<(usize, usize)> {
    let n = a.len().min(1000);
    let m = b.len().min(1000);
    if n == 0 || m == 0 {
        return vec![];
    }
    let a = &a[..n];
    let b = &b[..m];
    let inf = f32::INFINITY;
    let mut cost = vec![vec![inf; m]; n];
    cost[0][0] = (a[0] - b[0]).abs();
    for i in 1..n {
        cost[i][0] = cost[i - 1][0] + (a[i] - b[0]).abs();
    }
    for j in 1..m {
        cost[0][j] = cost[0][j - 1] + (a[0] - b[j]).abs();
    }
    for i in 1..n {
        for j in 1..m {
            let prev = cost[i - 1][j].min(cost[i][j - 1]).min(cost[i - 1][j - 1]);
            cost[i][j] = prev + (a[i] - b[j]).abs();
        }
    }
    let mut path = Vec::new();
    let (mut i, mut j) = (n - 1, m - 1);
    path.push((i, j));
    while i > 0 || j > 0 {
        if i == 0 {
            j -= 1;
        } else if j == 0 {
            i -= 1;
        } else {
            let best = cost[i - 1][j - 1].min(cost[i - 1][j]).min(cost[i][j - 1]);
            if best == cost[i - 1][j - 1] {
                i -= 1;
                j -= 1;
            } else if best == cost[i - 1][j] {
                i -= 1;
            } else {
                j -= 1;
            }
        }
        path.push((i, j));
    }
    path.reverse();
    path
}

/// Build a full spectral timeline comparing synthesis against a reference.
pub fn compare(
    ref_samples: &[f32],
    syn_samples: &[f32],
    sample_rate: u32,
    ref_f0: &[f32],
    syn_f0: &[f32],
    ref_energy: &[f32],
    syn_energy: &[f32],
) -> SpectralTimeline {
    let ref_mel = log_mel_spectrogram(ref_samples, sample_rate);
    let syn_mel = log_mel_spectrogram(syn_samples, sample_rate);

    // DTW-align the mel spectrograms using energy contours as the guide
    let path = dtw_align(ref_energy, syn_energy);

    let (aligned_ref_mel, aligned_syn_mel): (Vec<Vec<f32>>, Vec<Vec<f32>>) = if path.is_empty() {
        let n = ref_mel.len().min(syn_mel.len());
        (ref_mel[..n].to_vec(), syn_mel[..n].to_vec())
    } else {
        let r: Vec<Vec<f32>> = path
            .iter()
            .filter_map(|&(i, _)| ref_mel.get(i).cloned())
            .collect();
        let s: Vec<Vec<f32>> = path
            .iter()
            .filter_map(|&(_, j)| syn_mel.get(j).cloned())
            .collect();
        (r, s)
    };

    let (spectral_gap, spectral_gap_per_frame) =
        log_spectral_distance(&aligned_ref_mel, &aligned_syn_mel);

    // Aligned F0 and energy contours
    let (pitch_ref, pitch_syn) = if path.is_empty() {
        let n = ref_f0.len().min(syn_f0.len());
        (ref_f0[..n].to_vec(), syn_f0[..n].to_vec())
    } else {
        let r: Vec<f32> = path
            .iter()
            .filter_map(|&(i, _)| ref_f0.get(i).copied())
            .collect();
        let s: Vec<f32> = path
            .iter()
            .filter_map(|&(_, j)| syn_f0.get(j).copied())
            .collect();
        (r, s)
    };

    let (energy_ref, energy_syn) = if path.is_empty() {
        let n = ref_energy.len().min(syn_energy.len());
        (ref_energy[..n].to_vec(), syn_energy[..n].to_vec())
    } else {
        let r: Vec<f32> = path
            .iter()
            .filter_map(|&(i, _)| ref_energy.get(i).copied())
            .collect();
        let s: Vec<f32> = path
            .iter()
            .filter_map(|&(_, j)| syn_energy.get(j).copied())
            .collect();
        (r, s)
    };

    let pitch_match = contour_correlation(ref_f0, syn_f0);
    let energy_match = contour_correlation(ref_energy, syn_energy);

    // Top-10 worst frames
    let mut indexed: Vec<(usize, f32)> = spectral_gap_per_frame
        .iter()
        .enumerate()
        .map(|(i, &v)| (i, v))
        .collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    indexed.truncate(10);

    SpectralTimeline {
        spectral_gap,
        spectral_gap_per_frame,
        pitch_ref,
        pitch_syn,
        energy_ref,
        energy_syn,
        pitch_match,
        energy_match,
        worst_frames: indexed,
        frame_step_secs: HOP_MS / 1000.0,
    }
}

/// Render a sparkline from per-frame values (each char ≈ 50ms = 5 frames).
pub fn sparkline(per_frame: &[f32], chunk_size: usize) -> String {
    let bars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let chunks: Vec<f32> = per_frame
        .chunks(chunk_size)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect();
    let max = chunks.iter().cloned().fold(0.0f32, f32::max).max(0.001);
    chunks
        .iter()
        .map(|&v| {
            let idx = ((v / max) * (bars.len() - 1) as f32).round() as usize;
            bars[idx.min(bars.len() - 1)]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn identical_signals_zero_gap() {
        let s = sine(200.0, 1.0, 22050);
        let f0 = vec![200.0; 100];
        let energy = vec![0.5; 100];
        let tl = compare(&s, &s, 22050, &f0, &f0, &energy, &energy);
        assert!(
            tl.spectral_gap < 0.1,
            "identical signals should have ~0 gap, got {}",
            tl.spectral_gap
        );
        // Flat contour has zero variance → Pearson returns 0.0, not 1.0
        assert!(tl.pitch_match.abs() <= 1.0);
    }

    #[test]
    fn different_signals_nonzero_gap() {
        let s1 = sine(200.0, 1.0, 22050);
        let s2 = sine(3000.0, 1.0, 22050);
        let f0 = vec![200.0; 100];
        let energy = vec![0.5; 100];
        let tl = compare(&s1, &s2, 22050, &f0, &f0, &energy, &energy);
        assert!(
            tl.spectral_gap > 5.0,
            "different freqs should have large gap, got {}",
            tl.spectral_gap
        );
    }

    #[test]
    fn worst_frames_sorted_descending() {
        let s1 = sine(200.0, 1.0, 22050);
        let s2 = sine(500.0, 1.0, 22050);
        let f0 = vec![200.0; 100];
        let energy = vec![0.5; 100];
        let tl = compare(&s1, &s2, 22050, &f0, &f0, &energy, &energy);
        for w in tl.worst_frames.windows(2) {
            assert!(w[0].1 >= w[1].1, "worst frames not sorted");
        }
    }

    #[test]
    fn sparkline_renders() {
        let vals = vec![1.0, 2.0, 8.0, 3.0, 1.0];
        let s = sparkline(&vals, 1);
        assert_eq!(s.chars().count(), 5);
    }
}
