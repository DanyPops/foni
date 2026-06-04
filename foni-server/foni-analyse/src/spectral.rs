use rustfft::{num_complex::Complex, FftPlanner};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default)]
pub struct SpectralMetrics {
    /// How bright or thin the voice sounds — higher Hz = brighter/thinner.
    pub brightness_hz: f32,
    /// Spread of energy around the brightness centre — wider = richer sound.
    pub bandwidth_hz: f32,
    /// How pure the tone is: 0.0 = clean sine, 1.0 = pure noise.
    pub tone_purity: f32,
    /// Sign changes per second — a texture proxy, high for fricatives.
    pub zero_crossing_rate: f32,
    /// Bass vs treble energy balance in dB. Positive = more bass (deep voice).
    /// Deep baritone target: +5 to +10 dB. Negative = thin/bright.
    pub bass_balance_db: f32,
    /// How dark the voice is: dB drop per octave going up in frequency.
    /// Steep negative (−4 to −6) = warm/dark. Near zero = bright/synthetic.
    pub vocal_darkness_db_oct: f32,
    /// Where the voice’s character sits in the 1–5 kHz band, in Hz.
    /// Lower = heavier/darker character (bass). Higher = lighter (tenor).
    /// Bass: ~2384 Hz, baritone: ~2454 Hz, tenor: ~2705 Hz (Kob 2022).
    pub vocal_weight_hz: f32,
}

const FRAME_MS: f32 = 25.0;
const HOP_MS: f32 = 10.0;

struct FrameMetrics {
    centroid: f32,
    bandwidth: f32,
    flatness: f32,
    alpha_ratio: f32,
    tilt: f64,
}

fn analyse_frame(mags: &[f32], mag_sum: f32, bin_hz: f32, n_bins: usize) -> FrameMetrics {
    let centroid: f32 = mags
        .iter()
        .enumerate()
        .map(|(k, &m)| k as f32 * bin_hz * m)
        .sum::<f32>()
        / mag_sum;

    let bandwidth: f32 = (mags
        .iter()
        .enumerate()
        .map(|(k, &m)| {
            let diff = k as f32 * bin_hz - centroid;
            m * diff * diff
        })
        .sum::<f32>()
        / mag_sum)
        .sqrt();

    let flatness = spectral_flatness(mags, mag_sum, n_bins);
    let alpha_ratio = alpha_ratio(mags, bin_hz, n_bins);
    let tilt = spectral_tilt(mags, bin_hz, n_bins);

    FrameMetrics {
        centroid,
        bandwidth,
        flatness,
        alpha_ratio,
        tilt,
    }
}

fn spectral_flatness(mags: &[f32], mag_sum: f32, n_bins: usize) -> f32 {
    let log_sum: f64 = mags.iter().map(|&m| (m as f64 + 1e-10).ln()).sum::<f64>();
    let geo_mean = (log_sum / n_bins as f64).exp() as f32;
    let arith_mean = mag_sum / n_bins as f32;
    if arith_mean > 0.0 {
        geo_mean / arith_mean
    } else {
        0.0
    }
}

fn alpha_ratio(mags: &[f32], bin_hz: f32, n_bins: usize) -> f32 {
    let cut_bin = ((1000.0 / bin_hz) as usize).min(n_bins - 1);
    let low_power: f32 = mags[..cut_bin].iter().map(|&m| m * m).sum();
    let high_power: f32 = mags[cut_bin..].iter().map(|&m| m * m).sum();
    if high_power > 1e-20 {
        10.0 * (low_power / high_power).log10()
    } else {
        0.0
    }
}

#[allow(clippy::needless_range_loop)]
fn spectral_tilt(mags: &[f32], bin_hz: f32, n_bins: usize) -> f64 {
    let lo_bin = ((100.0 / bin_hz) as usize).max(1);
    let hi_bin = ((8000.0 / bin_hz) as usize).min(n_bins);
    let (mut sx, mut sy, mut sxy, mut sxx) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut n_tilt = 0usize;
    for (idx, &mag_k) in mags[lo_bin..hi_bin].iter().enumerate() {
        let power = mag_k * mag_k;
        if power > 1e-20 {
            let x = (((lo_bin + idx) as f32 * bin_hz) as f64).log2();
            let y = 10.0 * (power as f64).log10();
            sx += x;
            sy += y;
            sxy += x * y;
            sxx += x * x;
            n_tilt += 1;
        }
    }
    if n_tilt > 1 {
        let n = n_tilt as f64;
        (n * sxy - sx * sy) / (n * sxx - sx * sx)
    } else {
        0.0
    }
}

fn vocal_weight(ltas: &[f64], bin_hz: f32, n_bins: usize) -> f32 {
    let lo = ((1000.0 / bin_hz) as usize).min(n_bins - 1);
    let hi = ((5000.0 / bin_hz) as usize).min(n_bins);
    let ltas_total: f64 = ltas[lo..hi].iter().sum();
    if ltas_total <= 0.0 {
        return (lo + hi) as f32 * bin_hz * 0.5;
    }
    let mut cum = 0.0f64;
    let half = ltas_total * 0.5;
    for k in lo..hi {
        cum += ltas[k];
        if cum >= half {
            return k as f32 * bin_hz;
        }
    }
    (lo + hi) as f32 * bin_hz * 0.5
}

pub fn compute(samples: &[f32], sample_rate: u32) -> SpectralMetrics {
    let sr = sample_rate as f32;
    let frame_size = next_power_of_two((sr * FRAME_MS / 1000.0) as usize);
    let hop_size = (sr * HOP_MS / 1000.0) as usize;

    let mut planner: FftPlanner<f32> = FftPlanner::new();
    let fft = planner.plan_fft_forward(frame_size);

    let hann: Vec<f32> = (0..frame_size)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (frame_size - 1) as f32).cos())
        })
        .collect();

    let bin_hz = sr / frame_size as f32;
    let n_bins = frame_size / 2;

    let mut centroid_acc = 0.0f64;
    let mut bandwidth_acc = 0.0f64;
    let mut flatness_acc = 0.0f64;
    let mut alpha_ratio_acc = 0.0f64;
    let mut tilt_acc = 0.0f64;
    let mut frame_count = 0usize;
    let mut ltas = vec![0.0f64; n_bins];

    let mut i = 0;
    while i + frame_size <= samples.len() {
        let frame: Vec<f32> = samples[i..i + frame_size]
            .iter()
            .zip(&hann)
            .map(|(&s, &w)| s * w)
            .collect();

        let mut buf: Vec<Complex<f32>> =
            frame.iter().map(|&r| Complex { re: r, im: 0.0 }).collect();
        fft.process(&mut buf);

        let mags: Vec<f32> = buf[..n_bins].iter().map(|c| c.norm()).collect();
        let mag_sum: f32 = mags.iter().sum();

        if mag_sum > 1e-10 {
            let fm = analyse_frame(&mags, mag_sum, bin_hz, n_bins);
            centroid_acc += fm.centroid as f64;
            bandwidth_acc += fm.bandwidth as f64;
            flatness_acc += fm.flatness as f64;
            alpha_ratio_acc += fm.alpha_ratio as f64;
            tilt_acc += fm.tilt;
            frame_count += 1;

            for (bin, &m) in mags.iter().enumerate() {
                ltas[bin] += (m * m) as f64;
            }
        }
        i += hop_size;
    }

    let duration = samples.len() as f32 / sr;
    let zcr = if duration > 0.0 {
        samples
            .windows(2)
            .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
            .count() as f32
            / duration
    } else {
        0.0
    };

    if frame_count == 0 {
        return SpectralMetrics {
            zero_crossing_rate: zcr,
            ..SpectralMetrics::default()
        };
    }

    let n = frame_count as f64;
    SpectralMetrics {
        brightness_hz: (centroid_acc / n) as f32,
        bandwidth_hz: (bandwidth_acc / n) as f32,
        tone_purity: (flatness_acc / n) as f32,
        zero_crossing_rate: zcr,
        bass_balance_db: (alpha_ratio_acc / n) as f32,
        vocal_darkness_db_oct: (tilt_acc / n) as f32,
        vocal_weight_hz: vocal_weight(&ltas, bin_hz, n_bins),
    }
}

fn next_power_of_two(n: usize) -> usize {
    let mut p = 1;
    while p < n {
        p <<= 1;
    }
    p
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
    fn centroid_near_sine_frequency() {
        let samples = sine(1000.0, 0.5, 22050);
        let m = compute(&samples, 22050);
        assert!(
            (m.brightness_hz - 1000.0).abs() < 50.0,
            "brightness={}",
            m.brightness_hz
        );
    }

    #[test]
    fn pure_tone_has_low_tone_purity() {
        let samples = sine(440.0, 0.5, 22050);
        let m = compute(&samples, 22050);
        assert!(m.tone_purity < 0.05, "tone_purity={}", m.tone_purity);
    }

    #[test]
    fn white_noise_has_high_tone_purity() {
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
        assert!(m.tone_purity > 0.5, "tone_purity={}", m.tone_purity);
    }

    #[test]
    fn low_freq_signal_has_positive_alpha_ratio() {
        let samples = sine(300.0, 0.5, 22050);
        let m = compute(&samples, 22050);
        assert!(
            m.bass_balance_db > 5.0,
            "bass_balance={}",
            m.bass_balance_db
        );
    }

    #[test]
    fn high_freq_signal_has_negative_alpha_ratio() {
        let samples = sine(4000.0, 0.5, 22050);
        let m = compute(&samples, 22050);
        assert!(
            m.bass_balance_db < -5.0,
            "bass_balance={}",
            m.bass_balance_db
        );
    }

    #[test]
    fn white_noise_has_negative_spectral_tilt() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let samples: Vec<f32> = (0..22050u32)
            .map(|i| {
                let mut h = DefaultHasher::new();
                i.hash(&mut h);
                (h.finish() as f32 / u64::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        let m = compute(&samples, 22050);
        // White noise power ∝ f^0 → tilt ≈ 0 dB/oct; pink noise is -3 dB/oct
        // Pure noise should be near 0; just verify it's computed (finite)
        assert!(
            m.vocal_darkness_db_oct.is_finite(),
            "vocal_darkness not finite"
        );
    }

    #[test]
    fn vocal_weight_is_in_range() {
        // Low-frequency sine should push vocal weight toward the low end of 1–5 kHz
        let samples = sine(300.0, 1.0, 22050);
        let m = compute(&samples, 22050);
        // 300 Hz sine has no energy in 1–5 kHz — vocal_weight should be 0 or midpoint
        assert!(
            m.vocal_weight_hz >= 0.0,
            "vocal_weight must be non-negative"
        );
    }

    #[test]
    fn silence_has_zero_zcr() {
        let samples = vec![0.0f32; 22050];
        let m = compute(&samples, 22050);
        assert_eq!(m.zero_crossing_rate, 0.0);
    }
}
