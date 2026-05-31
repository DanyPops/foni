use serde::Serialize;

/// Silence threshold in linear amplitude. Equivalent to -40 dBFS.
const SILENCE_THRESHOLD: f32 = 0.01;

/// Minimum silent segment duration to count as a pause.
const PAUSE_MIN_SECS: f32 = 0.1;

#[derive(Debug, Clone, Serialize)]
pub struct TemporalMetrics {
    /// Total duration in seconds.
    pub duration_secs:      f32,
    /// Voiced (non-silent) frames per second — syllable tempo proxy.
    pub speech_rate:        f32,
    /// Number of silent segments longer than PAUSE_MIN_SECS.
    pub pause_count:        u32,
    /// Average duration of each pause in seconds. 0.0 if no pauses.
    pub mean_pause_duration: f32,
    /// Fraction of total duration spent in silence.
    pub pause_ratio:        f32,
}

pub fn compute(samples: &[f32], sample_rate: u32) -> TemporalMetrics {
    let sr = sample_rate as f32;
    let duration_secs = samples.len() as f32 / sr;

    let frame_size = (sr * 0.025) as usize; // 25ms frames
    let hop_size   = (sr * 0.010) as usize; // 10ms hop

    let mut voiced_frames = 0usize;
    let mut silent_frames = 0usize;
    let mut pauses: Vec<f32> = Vec::new();
    let mut current_silence_frames = 0usize;

    let mut i = 0;
    while i + frame_size <= samples.len() {
        let frame = &samples[i..i + frame_size];
        let rms = (frame.iter().map(|&s| s * s).sum::<f32>() / frame_size as f32).sqrt();

        if rms < SILENCE_THRESHOLD {
            silent_frames += 1;
            current_silence_frames += 1;
        } else {
            voiced_frames += 1;
            if current_silence_frames > 0 {
                let secs = current_silence_frames as f32 * (hop_size as f32 / sr);
                if secs >= PAUSE_MIN_SECS {
                    pauses.push(secs);
                }
                current_silence_frames = 0;
            }
        }
        i += hop_size;
    }
    // trailing silence
    if current_silence_frames > 0 {
        let secs = current_silence_frames as f32 * (hop_size as f32 / sr);
        if secs >= PAUSE_MIN_SECS {
            pauses.push(secs);
        }
    }

    let total_frames = voiced_frames + silent_frames;
    let speech_rate = if duration_secs > 0.0 {
        voiced_frames as f32 / duration_secs
    } else {
        0.0
    };
    let pause_ratio = if total_frames > 0 {
        silent_frames as f32 / total_frames as f32
    } else {
        0.0
    };
    let mean_pause_duration = if pauses.is_empty() {
        0.0
    } else {
        pauses.iter().sum::<f32>() / pauses.len() as f32
    };

    TemporalMetrics {
        duration_secs,
        speech_rate,
        pause_count: pauses.len() as u32,
        mean_pause_duration,
        pause_ratio,
    }
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

    fn silence(secs: f32, sr: u32) -> Vec<f32> {
        vec![0.0f32; (sr as f32 * secs) as usize]
    }

    #[test]
    fn duration_is_correct() {
        let samples = sine(440.0, 2.0, 22050);
        let m = compute(&samples, 22050);
        assert!((m.duration_secs - 2.0).abs() < 0.02);
    }

    #[test]
    fn pure_silence_has_zero_speech_rate() {
        let samples = silence(1.0, 22050);
        let m = compute(&samples, 22050);
        assert_eq!(m.speech_rate, 0.0);
    }

    #[test]
    fn detects_interior_pause() {
        // 0.5s speech, 0.3s silence, 0.5s speech
        let mut samples = sine(440.0, 0.5, 22050);
        samples.extend(silence(0.3, 22050));
        samples.extend(sine(440.0, 0.5, 22050));
        let m = compute(&samples, 22050);
        assert_eq!(m.pause_count, 1, "expected 1 pause, got {}", m.pause_count);
        assert!(m.mean_pause_duration > 0.1);
    }

    #[test]
    fn short_gap_below_threshold_not_counted() {
        // 0.5s speech, 50ms silence (< PAUSE_MIN_SECS), 0.5s speech
        let mut samples = sine(440.0, 0.5, 22050);
        samples.extend(silence(0.05, 22050));
        samples.extend(sine(440.0, 0.5, 22050));
        let m = compute(&samples, 22050);
        assert_eq!(m.pause_count, 0);
    }
}
