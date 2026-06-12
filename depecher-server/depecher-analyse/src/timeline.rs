use serde::{Deserialize, Serialize};

const FRAME_MS: f32 = 10.0;
const SILENCE_THR: f32 = 0.01; // -40 dBFS
/// Minimum silence duration to emit as a distinct pause segment.
const PAUSE_MIN_S: f32 = 0.05;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind {
    Voiced,
    Silence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub kind: SegmentKind,
    pub start_s: f32,
    pub end_s: f32,
    /// RMS level of this segment in dBFS.
    pub rms_db: f32,
    /// Mean F0 within voiced segment. 0.0 for silence.
    pub f0_hz: f32,
    /// Duration in seconds.
    pub duration_s: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    pub segments: Vec<Segment>,
    pub total_duration_s: f32,
    /// Number of silence segments >= PAUSE_MIN_S.
    pub pause_count: usize,
    /// Total silence time / total duration.
    pub silence_ratio: f32,
}

/// Segment a WAV into voiced and silence spans via energy-based VAD.
/// Deterministic — no ML. Identical input always produces identical output.
pub fn segment(samples: &[f32], sample_rate: u32) -> Timeline {
    let sr = sample_rate as f32;
    let frame_size = (sr * FRAME_MS / 1000.0) as usize;
    let total_duration_s = samples.len() as f32 / sr;

    if samples.is_empty() || frame_size == 0 {
        return Timeline {
            segments: vec![],
            total_duration_s: 0.0,
            pause_count: 0,
            silence_ratio: 0.0,
        };
    }

    // Classify each frame as voiced or silence
    let frame_count = samples.len() / frame_size;
    let mut voiced: Vec<bool> = Vec::with_capacity(frame_count);
    for i in 0..frame_count {
        let frame = &samples[i * frame_size..(i + 1) * frame_size];
        let rms = (frame.iter().map(|&s| s * s).sum::<f32>() / frame_size as f32).sqrt();
        voiced.push(rms >= SILENCE_THR);
    }

    // Merge consecutive frames of the same class into runs
    let mut segments: Vec<Segment> = Vec::new();
    let frame_s = FRAME_MS / 1000.0;
    let mut i = 0;
    while i < voiced.len() {
        let kind = voiced[i];
        let run_start = i;
        while i < voiced.len() && voiced[i] == kind {
            i += 1;
        }
        let run_end = i;

        let start_s = run_start as f32 * frame_s;
        let end_s = (run_end as f32 * frame_s).min(total_duration_s);
        let duration_s = end_s - start_s;

        // Skip sub-threshold silence (keep micro-gaps as voiced)
        if !kind && duration_s < PAUSE_MIN_S {
            // Re-classify these frames as voiced by merging into adjacent
            // segment — just emit a silence but don't count it as a pause later
        }

        // Compute RMS for this segment
        let sample_start = (run_start * frame_size).min(samples.len());
        let sample_end = (run_end * frame_size).min(samples.len());
        let seg_samples = &samples[sample_start..sample_end];
        let rms = if seg_samples.is_empty() {
            0.0f32
        } else {
            (seg_samples.iter().map(|&s| s * s).sum::<f32>() / seg_samples.len() as f32).sqrt()
        };
        let rms_db = if rms > 0.0 {
            20.0 * rms.log10()
        } else {
            f32::NEG_INFINITY
        };

        segments.push(Segment {
            kind: if kind {
                SegmentKind::Voiced
            } else {
                SegmentKind::Silence
            },
            start_s,
            end_s,
            rms_db,
            f0_hz: 0.0, // filled by caller if needed
            duration_s,
        });
    }

    let pause_count = segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Silence && s.duration_s >= PAUSE_MIN_S)
        .count();
    let total_silence_s: f32 = segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Silence)
        .map(|s| s.duration_s)
        .sum();
    let silence_ratio = if total_duration_s > 0.0 {
        total_silence_s / total_duration_s
    } else {
        0.0
    };

    Timeline {
        segments,
        total_duration_s,
        pause_count,
        silence_ratio,
    }
}

/// Merge voiced segments separated by silence shorter than `max_gap_s`.
/// This is VAD hangover — suppresses word-internal inter-phoneme gaps.
/// Standard value: 0.08–0.12 s (one or two phoneme-boundary silences).
pub fn merge_short_silences(tl: &Timeline, max_gap_s: f32) -> Timeline {
    let mut merged: Vec<Segment> = Vec::new();

    for seg in &tl.segments {
        match merged.last_mut() {
            Some(prev)
                if prev.kind == SegmentKind::Voiced
                    && seg.kind == SegmentKind::Silence
                    && seg.duration_s < max_gap_s =>
            {
                // Absorb short silence into the preceding voiced segment
                prev.end_s = seg.end_s;
                prev.duration_s = prev.end_s - prev.start_s;
            }
            Some(prev) if prev.kind == SegmentKind::Voiced && seg.kind == SegmentKind::Voiced => {
                // Merge consecutive voiced runs (shouldn't normally happen but be safe)
                prev.end_s = seg.end_s;
                prev.duration_s = prev.end_s - prev.start_s;
                // Update RMS: take max (loudest is most representative)
                if seg.rms_db > prev.rms_db {
                    prev.rms_db = seg.rms_db;
                }
            }
            _ => merged.push(seg.clone()),
        }
    }

    let pause_count = merged
        .iter()
        .filter(|s| s.kind == SegmentKind::Silence && s.duration_s >= PAUSE_MIN_S)
        .count();
    let total_silence_s: f32 = merged
        .iter()
        .filter(|s| s.kind == SegmentKind::Silence)
        .map(|s| s.duration_s)
        .sum();
    let silence_ratio = if tl.total_duration_s > 0.0 {
        total_silence_s / tl.total_duration_s
    } else {
        0.0
    };

    Timeline {
        segments: merged,
        total_duration_s: tl.total_duration_s,
        pause_count,
        silence_ratio,
    }
}

/// Return only the silence (pause) segments meeting the minimum duration threshold.
pub fn pauses(tl: &Timeline) -> Vec<&Segment> {
    tl.segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Silence && s.duration_s >= PAUSE_MIN_S)
        .collect()
}

/// Return only the voiced segments.
pub fn voiced_segments(tl: &Timeline) -> Vec<&Segment> {
    tl.segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Voiced)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn silence(secs: f32, sr: u32) -> Vec<f32> {
        vec![0.0f32; (sr as f32 * secs) as usize]
    }
    fn tone(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sr as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn pure_silence_has_no_voiced_segments() {
        let tl = segment(&silence(1.0, 22050), 22050);
        assert!(voiced_segments(&tl).is_empty());
    }

    #[test]
    fn pure_tone_has_no_silence() {
        let tl = segment(&tone(440.0, 1.0, 22050), 22050);
        assert!(pauses(&tl).is_empty());
    }

    #[test]
    fn silence_tone_silence_produces_three_segments() {
        let mut s = silence(0.3, 22050);
        s.extend(tone(440.0, 0.5, 22050));
        s.extend(silence(0.3, 22050));
        let tl = segment(&s, 22050);
        let v = voiced_segments(&tl);
        let p = pauses(&tl);
        assert!(!v.is_empty(), "expected voiced segments");
        assert_eq!(p.len(), 2, "expected 2 pauses (leading + trailing)");
    }

    #[test]
    fn interior_pause_detected() {
        // speech, pause, speech
        let mut s = tone(440.0, 0.4, 22050);
        s.extend(silence(0.2, 22050)); // 200ms pause > PAUSE_MIN_S
        s.extend(tone(440.0, 0.4, 22050));
        let tl = segment(&s, 22050);
        let p = pauses(&tl);
        assert!(!p.is_empty(), "expected interior pause");
        assert!(p[0].duration_s >= 0.15, "pause={:.3}s", p[0].duration_s);
    }

    #[test]
    fn short_gap_below_threshold_not_a_pause() {
        let mut s = tone(440.0, 0.4, 22050);
        s.extend(silence(0.02, 22050)); // 20ms < PAUSE_MIN_S
        s.extend(tone(440.0, 0.4, 22050));
        let tl = segment(&s, 22050);
        let long_pauses: Vec<_> = pauses(&tl)
            .into_iter()
            .filter(|p| p.duration_s >= PAUSE_MIN_S)
            .collect();
        assert!(long_pauses.is_empty(), "20ms gap should not be a pause");
    }

    #[test]
    fn total_duration_correct() {
        let s = tone(440.0, 2.0, 22050);
        let tl = segment(&s, 22050);
        assert!((tl.total_duration_s - 2.0).abs() < 0.02);
    }
}
