/// Dynamics processing: compressor, hard limiter, EBU R128 loudnorm.

use ebur128::{EbuR128, Mode};

// ─── Compressor ───────────────────────────────────────────────────────────────

pub struct Compressor {
    threshold: f32,  // linear
    ratio:     f32,
    attack:    f32,  // coefficient (α) per sample
    release:   f32,
    makeup:    f32,  // linear gain
    env:       f32,  // envelope follower state
}

impl Compressor {
    pub fn new(threshold_db: f32, ratio: f32, attack_ms: f32, release_ms: f32,
               makeup_db: f32, sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        Compressor {
            threshold: 10f32.powf(threshold_db / 20.),
            ratio,
            attack:  (-1. / (sr * attack_ms  / 1000.)).exp(),
            release: (-1. / (sr * release_ms / 1000.)).exp(),
            makeup:  10f32.powf(makeup_db / 20.),
            env:     0.,
        }
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            let abs = s.abs();
            // Envelope follower
            let coeff = if abs > self.env { self.attack } else { self.release };
            self.env = abs + coeff * (self.env - abs);

            // Gain computation
            let gain = if self.env > self.threshold {
                let over = self.env / self.threshold;
                self.threshold * over.powf(1. / self.ratio) / self.env
            } else {
                1.
            };
            *s *= gain * self.makeup;
        }
    }
}

// ─── Hard limiter ─────────────────────────────────────────────────────────────

/// Clip samples to ±ceiling (linear amplitude).
pub fn hard_clip(samples: &mut [f32], ceiling_db: f32) {
    let ceil = 10f32.powf(ceiling_db / 20.);
    for s in samples.iter_mut() {
        *s = s.clamp(-ceil, ceil);
    }
}

// ─── EBU R128 loudnorm ───────────────────────────────────────────────────────

/// Normalise to target_lufs using the ebur128 crate (single-pass, linear gain).
/// Falls back to identity if measurement fails.
pub fn loudnorm(samples: &mut [f32], sample_rate: u32, target_lufs: f32) {
    let mut meter = match EbuR128::new(1, sample_rate, Mode::I) {
        Ok(m) => m,
        Err(_) => return,
    };
    if meter.add_frames_f32(samples).is_err() { return; }
    let measured = match meter.loudness_global() {
        Ok(l) if l.is_finite() => l as f32,
        _ => return,
    };
    let gain = 10f32.powf((target_lufs - measured) / 20.);
    // Cap gain to avoid blowing up near-silence
    let gain = gain.min(40.); // +32 dB max boost
    for s in samples.iter_mut() { *s *= gain; }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n).map(|i| (2. * PI * freq * i as f32 / sr as f32).sin() * 0.8).collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|&x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    fn db(r: f32) -> f32 { if r > 0. { 20. * r.log10() } else { f32::NEG_INFINITY } }

    #[test]
    fn compressor_reduces_loud_signal() {
        let mut sig = sine(200., 1., 22050);
        let rms_before = rms(&sig);
        let mut comp = Compressor::new(-12., 3., 10., 80., 0., 22050);
        comp.process(&mut sig);
        assert!(rms(&sig) < rms_before, "compressor raised level");
    }

    #[test]
    fn hard_clip_limits_peaks() {
        let mut sig = vec![0.99f32, -0.99, 1.5, -1.5];
        hard_clip(&mut sig, -1.); // ≈ 0.891
        for s in &sig { assert!(s.abs() <= 0.892, "peak not clipped: {s}"); }
    }

    #[test]
    fn loudnorm_raises_quiet_signal() {
        let mut sig = sine(200., 2., 22050);
        for s in sig.iter_mut() { *s *= 0.05; } // very quiet
        let rms_before = rms(&sig);
        loudnorm(&mut sig, 22050, -14.);
        assert!(rms(&sig) > rms_before, "loudnorm didn't raise level");
    }

    #[test]
    fn loudnorm_clamps_gain() {
        // Near-silence should not blow up
        let mut sig = vec![1e-6f32; 44100];
        loudnorm(&mut sig, 22050, -14.);
        let peak = sig.iter().map(|&s| s.abs()).fold(0f32, f32::max);
        assert!(peak < 10., "gain exploded: peak={peak}");
    }
}
