use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VoiceMetrics {
    /// Harmonics-to-noise ratio in dB. Higher = cleaner voice. Natural speech: 15–25 dB.
    pub hnr_db: f32,
    /// Cepstral peak prominence — periodicity strength. Higher = more periodic.
    pub cpp: f32,
    /// Cycle-to-cycle F0 period perturbation (%). Natural speech: < 1%.
    pub jitter: f32,
    /// Cycle-to-cycle amplitude perturbation (%). Natural speech: < 5%.
    pub shimmer: f32,
}

const FRAME_MS: f32 = 25.0;
const HOP_MS: f32 = 10.0;
const SILENCE_THR: f32 = 0.01; // -40 dBFS

pub fn compute(samples: &[f32], sample_rate: u32) -> VoiceMetrics {
    let sr = sample_rate as f32;
    let frame_size = (sr * FRAME_MS / 1000.0) as usize;

    if samples.len() < frame_size * 2 {
        return VoiceMetrics {
            hnr_db: 0.0,
            cpp: 0.0,
            jitter: 0.0,
            shimmer: 0.0,
        };
    }

    VoiceMetrics {
        hnr_db: compute_hnr(samples, sample_rate, frame_size),
        cpp: compute_cpp(samples, sample_rate, frame_size),
        jitter: compute_jitter(samples, sample_rate),
        shimmer: compute_shimmer(samples, sample_rate),
    }
}

// ─── HNR via autocorrelation ──────────────────────────────────────────────────

fn compute_hnr(samples: &[f32], sample_rate: u32, frame_size: usize) -> f32 {
    let sr = sample_rate as usize;
    let hop = (sample_rate as f32 * HOP_MS / 1000.0) as usize;
    let min_lag = sr / 500; // 500 Hz max
    let max_lag = sr / 80; // 80 Hz min

    let mut hnr_sum = 0.0f64;
    let mut count = 0usize;

    let mut i = 0;
    while i + frame_size <= samples.len() {
        let frame = &samples[i..i + frame_size];
        let rms = (frame.iter().map(|&s| s * s).sum::<f32>() / frame_size as f32).sqrt();
        if rms < SILENCE_THR {
            i += hop;
            continue;
        }

        // Autocorrelation at lag 0 (total energy) and at the dominant lag
        let r0: f64 = frame.iter().map(|&s| (s as f64).powi(2)).sum();
        if r0 < 1e-12 {
            i += hop;
            continue;
        }

        let mut best_r = 0.0f64;
        for lag in min_lag..=max_lag.min(frame_size - 1) {
            let r: f64 = frame[..frame_size - lag]
                .iter()
                .zip(&frame[lag..])
                .map(|(&a, &b)| a as f64 * b as f64)
                .sum();
            if r > best_r {
                best_r = r;
            }
        }

        if best_r > 0.0 && r0 > best_r {
            let hnr_linear = best_r / (r0 - best_r);
            hnr_sum += 10.0 * hnr_linear.log10();
            count += 1;
        }
        i += hop;
    }

    if count == 0 {
        0.0
    } else {
        (hnr_sum / count as f64) as f32
    }
}

// ─── CPP (Cepstral Peak Prominence) ──────────────────────────────────────────

fn compute_cpp(samples: &[f32], sample_rate: u32, frame_size: usize) -> f32 {
    use std::f64::consts::PI;
    let sr = sample_rate as f32;
    let hop = (sr * HOP_MS / 1000.0) as usize;
    let min_quefrency = (sample_rate as f64 / 500.0) as usize; // 500 Hz max
    let max_quefrency = (sample_rate as f64 / 80.0) as usize; // 80 Hz min

    // Hann window
    let hann: Vec<f64> = (0..frame_size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f64 / (frame_size - 1) as f64).cos()))
        .collect();

    let mut cpp_sum = 0.0f64;
    let mut count = 0usize;
    let mut i = 0;

    while i + frame_size <= samples.len() {
        let frame = &samples[i..i + frame_size];
        let rms = (frame.iter().map(|&s| s * s).sum::<f32>() / frame_size as f32).sqrt();
        if rms < SILENCE_THR {
            i += hop;
            continue;
        }

        // Power spectrum log → real cepstrum (IDFT of log |X|)
        let windowed: Vec<f64> = frame
            .iter()
            .zip(&hann)
            .map(|(&s, &w)| s as f64 * w)
            .collect();
        // Simple power via Goertzel is too slow for all bins; use squared DFT magnitude estimate
        // For CPP we need the cepstrum — compute as IFFT(log(|FFT|²))
        // Approximate: use autocorrelation of log-power spectrum
        let log_power: Vec<f64> = {
            let n = frame_size;
            (0..n / 2)
                .map(|k| {
                    let re: f64 = windowed
                        .iter()
                        .enumerate()
                        .map(|(t, &x)| x * (2.0 * PI * k as f64 * t as f64 / n as f64).cos())
                        .sum();
                    let im: f64 = windowed
                        .iter()
                        .enumerate()
                        .map(|(t, &x)| -x * (2.0 * PI * k as f64 * t as f64 / n as f64).sin())
                        .sum();
                    ((re * re + im * im).max(1e-12)).ln()
                })
                .collect()
        };

        // Cepstrum: IDFT of log power
        let n = log_power.len();
        let cepstrum: Vec<f64> = (0..n)
            .map(|q| {
                log_power
                    .iter()
                    .enumerate()
                    .map(|(k, &lp)| lp * (2.0 * PI * k as f64 * q as f64 / n as f64).cos())
                    .sum::<f64>()
                    / n as f64
            })
            .collect();

        // CPP = cepstral peak in voiced range minus linear regression baseline
        let lo = min_quefrency.min(n - 1);
        let hi = max_quefrency.min(n - 1);
        if hi <= lo {
            i += hop;
            continue;
        }

        let peak = cepstrum[lo..=hi]
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        // Linear baseline at the quefrency of the peak
        let slope = (cepstrum[hi] - cepstrum[lo]) / (hi - lo) as f64;
        let peak_q = cepstrum[lo..=hi]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(q, _)| lo + q)
            .unwrap_or(lo);
        let baseline = cepstrum[lo] + slope * (peak_q - lo) as f64;

        cpp_sum += (peak - baseline).max(0.0);
        count += 1;
        i += hop;
    }

    if count == 0 {
        0.0
    } else {
        (cpp_sum / count as f64) as f32
    }
}

// ─── Jitter and Shimmer via zero-crossing period estimation ───────────────────

fn estimate_periods(samples: &[f32], sample_rate: u32) -> Vec<(f32, f32)> {
    // Returns (period_secs, rms_amplitude) for each detected cycle via zero crossings
    let sr = sample_rate as f32;
    let min_period = (sr / 500.0) as usize;
    let max_period = (sr / 80.0) as usize;

    let mut crossings: Vec<usize> = Vec::new();
    for i in 1..samples.len() {
        if samples[i - 1] <= 0.0 && samples[i] > 0.0 {
            // Interpolate for sub-sample precision
            crossings.push(i);
        }
    }

    let mut periods = Vec::new();
    let mut w = 0;
    while w + 1 < crossings.len() {
        let period = crossings[w + 1] - crossings[w];
        if period >= min_period && period <= max_period {
            let cycle = &samples[crossings[w]..crossings[w + 1]];
            let amp = (cycle.iter().map(|&s| s * s).sum::<f32>() / cycle.len() as f32).sqrt();
            if amp > SILENCE_THR {
                periods.push((period as f32 / sr, amp));
            }
        }
        w += 1;
    }
    periods
}

fn compute_jitter(samples: &[f32], sample_rate: u32) -> f32 {
    let periods = estimate_periods(samples, sample_rate);
    if periods.len() < 2 {
        return 0.0;
    }
    let periods_s: Vec<f32> = periods.iter().map(|(p, _)| *p).collect();
    let mean = periods_s.iter().sum::<f32>() / periods_s.len() as f32;
    if mean == 0.0 {
        return 0.0;
    }
    let sum_diff: f32 = periods_s.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
    (sum_diff / (periods_s.len() - 1) as f32) / mean
}

fn compute_shimmer(samples: &[f32], sample_rate: u32) -> f32 {
    let periods = estimate_periods(samples, sample_rate);
    if periods.len() < 2 {
        return 0.0;
    }
    let amps: Vec<f32> = periods.iter().map(|(_, a)| *a).collect();
    let mean = amps.iter().sum::<f32>() / amps.len() as f32;
    if mean == 0.0 {
        return 0.0;
    }
    let sum_diff: f32 = amps.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
    (sum_diff / (amps.len() - 1) as f32) / mean
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
    fn pure_sine_high_hnr() {
        let samples = sine(200.0, 1.0, 22050);
        let m = compute(&samples, 22050);
        assert!(
            m.hnr_db > 5.0,
            "hnr_db={} — expected > 5 for pure sine",
            m.hnr_db
        );
    }

    #[test]
    fn pure_sine_low_jitter() {
        let samples = sine(200.0, 1.0, 22050);
        let m = compute(&samples, 22050);
        assert!(
            m.jitter < 0.05,
            "jitter={} — expected near zero for perfect sine",
            m.jitter
        );
    }

    #[test]
    fn pure_sine_low_shimmer() {
        let samples = sine(200.0, 1.0, 22050);
        let m = compute(&samples, 22050);
        assert!(
            m.shimmer < 0.05,
            "shimmer={} — expected near zero for pure sine",
            m.shimmer
        );
    }

    #[test]
    fn silence_returns_zeros() {
        let samples = vec![0.0f32; 22050];
        let m = compute(&samples, 22050);
        assert_eq!(m.hnr_db, 0.0);
        assert_eq!(m.jitter, 0.0);
        assert_eq!(m.shimmer, 0.0);
    }
}
