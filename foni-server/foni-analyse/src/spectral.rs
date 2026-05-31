use rustfft::{FftPlanner, num_complex::Complex};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SpectralMetrics {
    /// Weighted mean frequency in Hz — perceptual brightness.
    pub centroid_hz:     f32,
    /// Weighted std dev around centroid in Hz.
    pub bandwidth_hz:    f32,
    /// Wiener entropy: 0.0 = pure tone, 1.0 = white noise.
    pub flatness:        f32,
    /// Sign changes per second — high for noise/fricatives, low for voiced.
    pub zero_crossing_rate: f32,
}

const FRAME_MS: f32  = 25.0;
const HOP_MS:   f32  = 10.0;

pub fn compute(samples: &[f32], sample_rate: u32) -> SpectralMetrics {
    let sr = sample_rate as f32;
    let frame_size = next_power_of_two((sr * FRAME_MS / 1000.0) as usize);
    let hop_size   = (sr * HOP_MS   / 1000.0) as usize;

    let mut planner: FftPlanner<f32> = FftPlanner::new();
    let fft = planner.plan_fft_forward(frame_size);

    let hann: Vec<f32> = (0..frame_size)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (frame_size - 1) as f32).cos()))
        .collect();

    let bin_hz = sr / frame_size as f32;
    let n_bins = frame_size / 2;

    let mut centroid_acc  = 0.0f64;
    let mut bandwidth_acc = 0.0f64;
    let mut flatness_acc  = 0.0f64;
    let mut frame_count   = 0usize;

    let mut i = 0;
    while i + frame_size <= samples.len() {
        let frame: Vec<f32> = samples[i..i + frame_size]
            .iter()
            .zip(&hann)
            .map(|(&s, &w)| s * w)
            .collect();

        let mut buf: Vec<Complex<f32>> = frame.iter().map(|&r| Complex { re: r, im: 0.0 }).collect();
        fft.process(&mut buf);

        let mags: Vec<f32> = buf[..n_bins].iter().map(|c| c.norm()).collect();
        let mag_sum: f32 = mags.iter().sum();

        if mag_sum > 1e-10 {
            let centroid: f32 = mags.iter().enumerate()
                .map(|(k, &m)| k as f32 * bin_hz * m)
                .sum::<f32>() / mag_sum;

            let bandwidth: f32 = (mags.iter().enumerate()
                .map(|(k, &m)| {
                    let diff = k as f32 * bin_hz - centroid;
                    m * diff * diff
                })
                .sum::<f32>() / mag_sum)
                .sqrt();

            // Spectral flatness (Wiener entropy)
            let log_sum: f64 = mags.iter()
                .map(|&m| (m as f64 + 1e-10).ln())
                .sum::<f64>();
            let geo_mean = (log_sum / n_bins as f64).exp() as f32;
            let arith_mean = mag_sum / n_bins as f32;
            let flatness = if arith_mean > 0.0 { geo_mean / arith_mean } else { 0.0 };

            centroid_acc  += centroid as f64;
            bandwidth_acc += bandwidth as f64;
            flatness_acc  += flatness as f64;
            frame_count   += 1;
        }
        i += hop_size;
    }

    let zero_crossings = samples.windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count();
    let duration = samples.len() as f32 / sr;
    let zcr = if duration > 0.0 { zero_crossings as f32 / duration } else { 0.0 };

    if frame_count == 0 {
        return SpectralMetrics { centroid_hz: 0.0, bandwidth_hz: 0.0, flatness: 0.0, zero_crossing_rate: zcr };
    }

    SpectralMetrics {
        centroid_hz:        (centroid_acc  / frame_count as f64) as f32,
        bandwidth_hz:       (bandwidth_acc / frame_count as f64) as f32,
        flatness:           (flatness_acc  / frame_count as f64) as f32,
        zero_crossing_rate: zcr,
    }
}

fn next_power_of_two(n: usize) -> usize {
    let mut p = 1;
    while p < n { p <<= 1; }
    p
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
    fn centroid_near_sine_frequency() {
        let samples = sine(1000.0, 0.5, 22050);
        let m = compute(&samples, 22050);
        assert!((m.centroid_hz - 1000.0).abs() < 50.0, "centroid={}", m.centroid_hz);
    }

    #[test]
    fn pure_tone_has_low_flatness() {
        let samples = sine(440.0, 0.5, 22050);
        let m = compute(&samples, 22050);
        assert!(m.flatness < 0.05, "flatness={}", m.flatness);
    }

    #[test]
    fn white_noise_has_high_flatness() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        // deterministic pseudo-noise
        let samples: Vec<f32> = (0..22050u32)
            .map(|i| {
                let mut h = DefaultHasher::new();
                i.hash(&mut h);
                (h.finish() as f32 / u64::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        let m = compute(&samples, 22050);
        assert!(m.flatness > 0.5, "flatness={}", m.flatness);
    }

    #[test]
    fn silence_has_zero_zcr() {
        let samples = vec![0.0f32; 22050];
        let m = compute(&samples, 22050);
        assert_eq!(m.zero_crossing_rate, 0.0);
    }
}
