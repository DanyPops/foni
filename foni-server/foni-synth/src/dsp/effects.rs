/// Time-domain effects: fade, silence trim, reverb (echo), vibrato.

const SILENCE_THR: f32 = 0.01; // -40 dBFS
const PAUSE_MIN_S: f32 = 0.05;

// ─── Fade ─────────────────────────────────────────────────────────────────────

pub fn fade_in(samples: &mut [f32], duration_s: f32, sample_rate: u32) {
    let fade_len = ((sample_rate as f32 * duration_s) as usize).min(samples.len());
    for i in 0..fade_len {
        samples[i] *= i as f32 / fade_len as f32;
    }
}

pub fn fade_out(samples: &mut [f32], duration_s: f32, sample_rate: u32) {
    let n = samples.len();
    let fade_len = ((sample_rate as f32 * duration_s) as usize).min(n);
    for i in 0..fade_len {
        samples[n - 1 - i] *= i as f32 / fade_len as f32;
    }
}

// ─── Silence trim (two-pass, preserves interior pauses) ──────────────────────

/// Remove leading silence below threshold.
fn trim_leading(samples: &[f32], frame_ms: f32, sr: u32) -> usize {
    let frame = (sr as f32 * frame_ms / 1000.) as usize;
    let mut start = 0usize;
    let mut i = 0usize;
    while i + frame <= samples.len() {
        let rms = (samples[i..i+frame].iter().map(|&s| s*s).sum::<f32>() / frame as f32).sqrt();
        if rms >= SILENCE_THR { break; }
        start = i + frame;
        i += frame;
    }
    start
}

/// Trim leading and trailing silence. Interior pauses are preserved.
pub fn silence_trim(samples: Vec<f32>, threshold_db: f32, sample_rate: u32) -> Vec<f32> {
    if threshold_db >= 0. { return samples; }  // disabled
    let frame_ms = 10.;
    // Leading trim
    let lead = trim_leading(&samples, frame_ms, sample_rate);
    // Trailing trim: reverse, trim, reverse back
    let mut rev: Vec<f32> = samples[lead..].iter().copied().rev().collect();
    let trail_start = trim_leading(&rev, frame_ms, sample_rate);
    rev.drain(..trail_start);
    rev.iter().copied().rev().collect()
}

// ─── Simple echo/reverb (aecho equivalent) ───────────────────────────────────

/// Single-tap delay echo (approximates ffmpeg aecho).
pub fn echo(samples: &mut [f32], delay_ms: f32, decay: f32,
            in_gain: f32, out_gain: f32, sample_rate: u32) {
    if delay_ms <= 0. || decay <= 0. { return; }
    let delay = (sample_rate as f32 * delay_ms / 1000.) as usize;
    let mut buf = vec![0f32; delay];
    let mut head = 0usize;
    for s in samples.iter_mut() {
        let echo_out = buf[head] * out_gain;
        buf[head] = *s * in_gain + echo_out * decay;
        head = (head + 1) % delay;
        *s += echo_out;
    }
}

// ─── Vibrato (sinusoidal pitch micro-variation via interpolated delay) ─────────

/// Vibrato via LFO-modulated delay line. Depth in fractional samples.
pub fn vibrato(samples: &mut [f32], freq_hz: f32, depth: f32, sample_rate: u32) {
    if freq_hz <= 0. || depth <= 0. { return; }
    let sr = sample_rate as f32;
    let max_delay = (depth * sr).ceil() as usize + 2;
    let mut buf = vec![0f32; max_delay];
    let mut head = 0usize;

    for (i, s) in samples.iter_mut().enumerate() {
        buf[head] = *s;
        let lfo = (2. * std::f32::consts::PI * freq_hz * i as f32 / sr).sin();
        let delay_f = depth * sr * (1. + lfo) / 2.; // 0..depth*sr
        let delay_i = delay_f as usize;
        let frac    = delay_f - delay_i as f32;
        let idx0 = (head + max_delay - delay_i    ) % max_delay;
        let idx1 = (head + max_delay - delay_i - 1) % max_delay;
        *s = buf[idx0] * (1. - frac) + buf[idx1] * frac;
        head = (head + 1) % max_delay;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silence(secs: f32, sr: u32) -> Vec<f32> { vec![0f32; (sr as f32 * secs) as usize] }

    fn tone(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n).map(|i| (2. * std::f32::consts::PI * freq * i as f32 / sr as f32).sin() * 0.5).collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|&x| x*x).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn fade_in_starts_near_zero() {
        let mut sig = tone(440., 0.5, 22050);
        fade_in(&mut sig, 0.04, 22050);
        let first_5ms = &sig[..((22050. * 0.005) as usize)];
        assert!(rms(first_5ms) < 0.05, "fade-in not applied: rms={}", rms(first_5ms));
    }

    #[test]
    fn silence_trim_removes_edges() {
        let mut s = silence(0.3, 22050);
        s.extend(tone(440., 0.5, 22050));
        s.extend(silence(0.3, 22050));
        let trimmed = silence_trim(s, -40., 22050);
        // Trimmed should be shorter (leading + trailing silence gone)
        assert!(trimmed.len() < (22050. * 1.1) as usize, "silence not trimmed");
        assert!(trimmed.len() > (22050. * 0.4) as usize, "too much trimmed");
    }

    #[test]
    fn vibrato_preserves_approximate_level() {
        let mut sig = tone(200., 1., 22050);
        let rms_before = rms(&sig);
        vibrato(&mut sig, 6., 0.003, 22050);
        let rms_after = rms(&sig);
        // Level should not change drastically
        assert!((rms_after - rms_before).abs() < 0.05, "vibrato changed level too much");
    }
}
