use serde::{Deserialize, Serialize};

/// One word or pause from the whisper-timestamped fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefWord {
    pub word:       String,
    pub start_s:    f32,
    pub end_s:      f32,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefPause {
    pub after_word: String,
    pub start_s:    f32,
    pub end_s:      f32,
    pub duration_s: f32,
}

/// The fixture file format produced by scripts/extract-timeline.py.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineFixture {
    pub _source:          String,
    pub _model:           String,
    pub total_duration_s: f32,
    pub words:            Vec<RefWord>,
    pub pauses:           Vec<RefPause>,
}

/// One aligned pair (word or pause) with deviation metrics.
#[derive(Debug, Clone, Serialize)]
pub struct AlignedPair {
    /// Word text or "[pause]".
    pub label:          String,
    pub ref_start_s:    f32,
    pub ref_end_s:      f32,
    pub ref_duration_s: f32,
    pub syn_start_s:    f32,
    pub syn_end_s:      f32,
    pub syn_duration_s: f32,
    /// syn_duration / ref_duration. 1.0 = identical. < 1.0 = synthetic is shorter.
    pub duration_ratio: f32,
    /// Synthetic RMS minus reference RMS in dB. 0 = identical level.
    pub rms_delta_db:   f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineComparison {
    pub pairs:              Vec<AlignedPair>,
    /// Mean duration_ratio across all pairs.
    pub mean_duration_ratio: f32,
    /// Duration ratio of the shortest pause relative to its reference.
    /// 0.0 when no pauses; < 0.3 is the voiced-ratio problem.
    pub worst_pause_ratio:  f32,
    /// Mean RMS delta in dB across all word pairs.
    pub mean_rms_delta_db:  f32,
}

/// Align a reference timeline fixture to a VAD-segmented synthetic timeline.
///
/// Alignment is by sequence position, not DTW — same phrase, same word order.
/// The synthetic timeline is a flat Vec<(start_s, end_s, rms_db)> of speech runs
/// extracted by the caller from `crate::timeline::voiced_segments()`.
pub fn align(
    fixture:    &TimelineFixture,
    syn_voiced: &[(f32, f32, f32)],  // (start_s, end_s, rms_db)
    syn_pauses: &[(f32, f32, f32)],  // (start_s, end_s, rms_db)
) -> TimelineComparison {
    let mut pairs: Vec<AlignedPair> = Vec::new();

    // ── Word pairs ────────────────────────────────────────────────────────────
    for (i, ref_word) in fixture.words.iter().enumerate() {
        let ref_dur = ref_word.end_s - ref_word.start_s;
        if let Some(&(ss, se, srms)) = syn_voiced.get(i) {
            let syn_dur = se - ss;
            let ratio   = if ref_dur > 0.0 { syn_dur / ref_dur } else { 1.0 };
            // Reference RMS not stored in fixture; use synthetic RMS delta vs 0
            pairs.push(AlignedPair {
                label:          ref_word.word.clone(),
                ref_start_s:    ref_word.start_s,
                ref_end_s:      ref_word.end_s,
                ref_duration_s: ref_dur,
                syn_start_s:    ss,
                syn_end_s:      se,
                syn_duration_s: syn_dur,
                duration_ratio: (ratio * 1000.0).round() / 1000.0,
                rms_delta_db:   srms, // delta from target; caller provides
            });
        }
    }

    // ── Pause pairs ───────────────────────────────────────────────────────────
    for (i, ref_pause) in fixture.pauses.iter().enumerate() {
        let ref_dur = ref_pause.duration_s;
        if let Some(&(ss, se, srms)) = syn_pauses.get(i) {
            let syn_dur = se - ss;
            let ratio   = if ref_dur > 0.0 { syn_dur / ref_dur } else { 1.0 };
            pairs.push(AlignedPair {
                label:          format!("[pause after '{}']", ref_pause.after_word),
                ref_start_s:    ref_pause.start_s,
                ref_end_s:      ref_pause.end_s,
                ref_duration_s: ref_dur,
                syn_start_s:    ss,
                syn_end_s:      se,
                syn_duration_s: syn_dur,
                duration_ratio: (ratio * 1000.0).round() / 1000.0,
                rms_delta_db:   srms,
            });
        } else {
            // Pause present in reference but absent from synthesis
            pairs.push(AlignedPair {
                label:          format!("[pause after '{}'] MISSING", ref_pause.after_word),
                ref_start_s:    ref_pause.start_s,
                ref_end_s:      ref_pause.end_s,
                ref_duration_s: ref_dur,
                syn_start_s:    0.0,
                syn_end_s:      0.0,
                syn_duration_s: 0.0,
                duration_ratio: 0.0,
                rms_delta_db:   0.0,
            });
        }
    }

    // ── Summary ───────────────────────────────────────────────────────────────
    let mean_duration_ratio = if pairs.is_empty() { 1.0 } else {
        pairs.iter().map(|p| p.duration_ratio).sum::<f32>() / pairs.len() as f32
    };
    let pause_pairs: Vec<_> = pairs.iter()
        .filter(|p| p.label.starts_with("[pause"))
        .collect();
    let worst_pause_ratio = pause_pairs.iter()
        .map(|p| p.duration_ratio)
        .fold(f32::INFINITY, f32::min);
    let worst_pause_ratio = if worst_pause_ratio.is_infinite() { 0.0 } else { worst_pause_ratio };

    let word_pairs: Vec<_> = pairs.iter().filter(|p| !p.label.starts_with("[pause")).collect();
    let mean_rms_delta_db = if word_pairs.is_empty() { 0.0 } else {
        word_pairs.iter().map(|p| p.rms_delta_db).sum::<f32>() / word_pairs.len() as f32
    };

    TimelineComparison { pairs, mean_duration_ratio, worst_pause_ratio, mean_rms_delta_db }
}

/// Render a comparison as a fixed-width table (mirrors formatGapTable style).
pub fn format_alignment_table(cmp: &TimelineComparison) -> String {
    let sep = "─".repeat(72);
    let mut lines = vec![
        format!("{:<30} {:>8}  {:>8}  {:>8}  {:>6}", "Segment", "Ref(ms)", "Syn(ms)", "Ratio", "Status"),
        sep.clone(),
    ];
    for p in &cmp.pairs {
        let status = if p.duration_ratio == 0.0 {
            "❌ MISSING"
        } else if p.label.starts_with("[pause") {
            if p.duration_ratio >= 0.5 { "✅ ok" }
            else if p.duration_ratio >= 0.3 { "🟡 short" }
            else { "🔴 collapsed" }
        } else {
            if p.duration_ratio >= 0.6 && p.duration_ratio <= 1.8 { "✅ ok" }
            else if p.duration_ratio < 0.6 { "🟠 too short" }
            else { "🟠 too long" }
        };
        lines.push(format!(
            "{:<30} {:>8.0}  {:>8.0}  {:>8.3}  {}",
            truncate(&p.label, 30),
            p.ref_duration_s * 1000.0,
            p.syn_duration_s * 1000.0,
            p.duration_ratio,
            status,
        ));
    }
    lines.push(sep);
    lines.push(format!(
        "Mean ratio: {:.3}   Worst pause: {:.3}   Mean RMS Δ: {:+.1}dB",
        cmp.mean_duration_ratio, cmp.worst_pause_ratio, cmp.mean_rms_delta_db,
    ));
    lines.join("\n")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() }
    else { format!("{}…", &s[..max.saturating_sub(1)]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> TimelineFixture {
        TimelineFixture {
            _source: "test.wav".into(), _model: "base".into(),
            total_duration_s: 3.0,
            words: vec![
                RefWord { word: "Подойди".into(), start_s: 0.0, end_s: 0.4, confidence: 0.9 },
                RefWord { word: "ка".into(),      start_s: 0.5, end_s: 0.7, confidence: 0.9 },
            ],
            pauses: vec![
                RefPause { after_word: "Подойди".into(), start_s: 0.4, end_s: 0.5, duration_s: 0.1 },
            ],
        }
    }

    #[test]
    fn identical_timelines_ratio_one() {
        let voiced = vec![(0.0, 0.4, 0.0), (0.5, 0.7, 0.0)];
        let pauses  = vec![(0.4, 0.5, 0.0)];
        let cmp = align(&fixture(), &voiced, &pauses);
        assert!((cmp.mean_duration_ratio - 1.0).abs() < 0.01);
    }

    #[test]
    fn missing_pause_gets_zero_ratio() {
        let voiced = vec![(0.0, 0.4, 0.0), (0.4, 0.6, 0.0)];
        let pauses: Vec<(f32,f32,f32)> = vec![];
        let cmp = align(&fixture(), &voiced, &pauses);
        assert_eq!(cmp.worst_pause_ratio, 0.0);
        let pause_pair = cmp.pairs.iter().find(|p| p.label.contains("MISSING"));
        assert!(pause_pair.is_some());
    }

    #[test]
    fn short_pause_low_ratio() {
        let voiced = vec![(0.0, 0.4, 0.0), (0.41, 0.61, 0.0)];
        let pauses  = vec![(0.40, 0.41, 0.0)]; // 10ms vs 100ms reference
        let cmp = align(&fixture(), &voiced, &pauses);
        assert!(cmp.worst_pause_ratio < 0.2, "ratio={}", cmp.worst_pause_ratio);
    }
}
