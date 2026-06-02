//! Tempo comparison — which parts of the utterance are rushed or drawn out.
//!
//! Segments both recordings into voiced/silence spans, aligns them via DTW,
//! then compares durations segment by segment.

use crate::timeline::{segment, SegmentKind, Timeline};
use serde::Serialize;

/// One aligned segment pair: reference duration vs synthesis duration.
#[derive(Debug, Clone, Serialize)]
pub struct TempoMatch {
    /// Which segment in the utterance (0-indexed).
    pub index: usize,
    /// Whether this is a voiced or silent segment.
    pub kind: SegmentKind,
    /// Duration in the studio reference (seconds).
    pub ref_duration_s: f32,
    /// Duration in the synthesis (seconds).
    pub syn_duration_s: f32,
    /// Ratio: synthesis / reference. 1.0 = matched. < 1.0 = rushed. > 1.0 = drawn out.
    pub ratio: f32,
    /// Start time in the reference (seconds).
    pub ref_start_s: f32,
}

/// Full tempo comparison between a reference and synthesis recording.
#[derive(Debug, Clone, Serialize)]
pub struct TempoComparison {
    /// All aligned segment pairs.
    pub pairs: Vec<TempoMatch>,
    /// Overall speed ratio: synthesis total / reference total. 1.0 = same duration.
    pub overall_ratio: f32,
    /// Segments where the synthesis is significantly faster (ratio < 0.7).
    pub rushed: Vec<TempoMatch>,
    /// Segments where the synthesis is significantly slower (ratio > 1.4).
    pub drawn_out: Vec<TempoMatch>,
}

/// Compare the pacing of two recordings by aligning their voiced/silence segments.
pub fn compare(ref_samples: &[f32], syn_samples: &[f32], sample_rate: u32) -> TempoComparison {
    let ref_tl = segment(ref_samples, sample_rate);
    let syn_tl = segment(syn_samples, sample_rate);

    let ref_durs: Vec<f32> = ref_tl.segments.iter().map(|s| s.duration_s).collect();
    let syn_durs: Vec<f32> = syn_tl.segments.iter().map(|s| s.duration_s).collect();

    let path = dtw_path(&ref_durs, &syn_durs);

    let mut pairs = Vec::new();
    let mut seen_ref = std::collections::HashSet::new();

    for &(ri, si) in &path {
        if seen_ref.contains(&ri) {
            continue;
        }
        seen_ref.insert(ri);

        let ref_seg = &ref_tl.segments[ri];
        let syn_seg = &syn_tl.segments[si.min(syn_tl.segments.len() - 1)];
        let ratio = if ref_seg.duration_s > 0.01 {
            syn_seg.duration_s / ref_seg.duration_s
        } else {
            1.0
        };

        pairs.push(TempoMatch {
            index: ri,
            kind: ref_seg.kind.clone(),
            ref_duration_s: ref_seg.duration_s,
            syn_duration_s: syn_seg.duration_s,
            ratio,
            ref_start_s: ref_seg.start_s,
        });
    }

    let overall_ratio = if ref_tl.total_duration_s > 0.0 {
        syn_tl.total_duration_s / ref_tl.total_duration_s
    } else {
        1.0
    };

    let rushed: Vec<TempoMatch> = pairs
        .iter()
        .filter(|p| p.ratio < 0.7 && matches!(p.kind, SegmentKind::Voiced))
        .cloned()
        .collect();

    let drawn_out: Vec<TempoMatch> = pairs
        .iter()
        .filter(|p| p.ratio > 1.4 && matches!(p.kind, SegmentKind::Voiced))
        .cloned()
        .collect();

    TempoComparison {
        pairs,
        overall_ratio,
        rushed,
        drawn_out,
    }
}

fn dtw_path(a: &[f32], b: &[f32]) -> Vec<(usize, usize)> {
    let n = a.len();
    let m = b.len();
    if n == 0 || m == 0 {
        return vec![];
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine_with_gap(sr: u32) -> Vec<f32> {
        let mut samples = Vec::new();
        // 0.5s voiced
        for i in 0..(sr as f32 * 0.5) as usize {
            samples.push((2.0 * PI * 200.0 * i as f32 / sr as f32).sin() * 0.3);
        }
        // 0.2s silence
        samples.extend(vec![0.0f32; (sr as f32 * 0.2) as usize]);
        // 0.5s voiced
        for i in 0..(sr as f32 * 0.5) as usize {
            samples.push((2.0 * PI * 200.0 * i as f32 / sr as f32).sin() * 0.3);
        }
        samples
    }

    #[test]
    fn identical_signals_have_ratio_near_one() {
        let s = sine_with_gap(22050);
        let tc = compare(&s, &s, 22050);
        assert!(
            (tc.overall_ratio - 1.0).abs() < 0.05,
            "overall_ratio={}",
            tc.overall_ratio
        );
        assert!(tc.rushed.is_empty());
        assert!(tc.drawn_out.is_empty());
    }

    #[test]
    fn faster_synthesis_detected_as_rushed() {
        let sr = 22050u32;
        let reference = sine_with_gap(sr);
        // Compress to 70% duration via resampling
        let ratio = 0.7;
        let out_len = (reference.len() as f64 * ratio) as usize;
        let faster: Vec<f32> = (0..out_len)
            .map(|i| {
                let pos = i as f64 / ratio;
                let lo = pos.floor() as usize;
                let hi = (lo + 1).min(reference.len() - 1);
                let frac = (pos - lo as f64) as f32;
                reference[lo] * (1.0 - frac) + reference[hi] * frac
            })
            .collect();
        let tc = compare(&reference, &faster, sr);
        assert!(
            tc.overall_ratio < 0.85,
            "overall_ratio={} should be <0.85",
            tc.overall_ratio
        );
    }

    #[test]
    fn pairs_are_not_empty() {
        let s = sine_with_gap(22050);
        let tc = compare(&s, &s, 22050);
        assert!(!tc.pairs.is_empty());
    }
}
