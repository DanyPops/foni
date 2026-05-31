use serde::{Deserialize, Serialize};
use crate::AnalysisResult;

// ─── Target tensor ────────────────────────────────────────────────────────────

/// Desired-state acoustic fingerprint — loaded from baseline/target.json
/// or built from a single reference WAV via TargetTensor::from_analysis().
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetTensor {
    pub _description: String,
    pub _sources:     Vec<String>,
    pub temporal: TargetTemporal,
    pub spectral:  TargetSpectral,
    pub voice:     TargetVoice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetTemporal {
    pub duration_secs:       f32,
    pub speech_rate:         f32,
    pub pause_count:         f32,
    pub mean_pause_duration: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetSpectral {
    pub rms_db:          f32,
    pub crest_factor:    f32,
    pub centroid_hz:     f32,
    pub flatness:        f32,
    pub zero_crossing_rate: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetVoice {
    pub f0_mean_hz:   f32,
    pub f0_stddev_hz: f32,
    pub voiced_ratio: f32,
}

impl TargetTensor {
    /// Build a tensor from a single reference analysis — no averaging.
    pub fn from_analysis(a: &AnalysisResult, label: &str) -> Self {
        TargetTensor {
            _description: label.to_string(),
            _sources:     vec![label.to_string()],
            temporal: TargetTemporal {
                duration_secs:       a.temporal.duration_secs,
                speech_rate:         a.temporal.speech_rate,
                pause_count:         a.temporal.pause_count as f32,
                mean_pause_duration: a.temporal.mean_pause_duration,
            },
            spectral: TargetSpectral {
                rms_db:              a.loudness.rms_db,
                crest_factor:        a.loudness.crest_factor,
                centroid_hz:         a.spectral.centroid_hz,
                flatness:            a.spectral.flatness,
                zero_crossing_rate:  a.spectral.zero_crossing_rate,
            },
            voice: TargetVoice {
                f0_mean_hz:   a.pitch.f0_mean_hz,
                f0_stddev_hz: a.pitch.f0_stddev_hz,
                voiced_ratio: a.pitch.voiced_ratio,
            },
        }
    }
}

// ─── Gap types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub enum Verdict {
    #[serde(rename = "✅ close")]    Close,
    #[serde(rename = "🟡 near")]     Near,
    #[serde(rename = "🟠 far")]      Far,
    #[serde(rename = "🔴 very far")] VeryFar,
}

impl Verdict {
    pub fn for_gap(pct: f32) -> Self {
        if pct < 15.0      { Verdict::Close }
        else if pct < 35.0 { Verdict::Near }
        else if pct < 60.0 { Verdict::Far }
        else               { Verdict::VeryFar }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Close   => "✅ close",
            Verdict::Near    => "🟡 near",
            Verdict::Far     => "🟠 far",
            Verdict::VeryFar => "🔴 very far",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GapRow {
    pub metric:  String,
    pub target:  String,
    pub actual:  String,
    pub gap_pct: f32,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Serialize)]
pub struct GapResult {
    pub phrase:      String,
    pub rows:        Vec<GapRow>,
    pub mean_gap_pct: f32,
}

// ─── Metric definitions ───────────────────────────────────────────────────────

struct MetricDef {
    name:   &'static str,
    target: fn(&TargetTensor) -> f32,
    actual: fn(&AnalysisResult) -> f32,
    unit:   &'static str,
    scale:  f32,
}

const METRICS: &[MetricDef] = &[
    MetricDef { name: "RMS level",          target: |t| t.spectral.rms_db,              actual: |a| a.loudness.rms_db,              unit: " dBFS", scale: 10.0 },
    MetricDef { name: "Crest factor",       target: |t| t.spectral.crest_factor,         actual: |a| a.loudness.crest_factor,         unit: " dB",   scale: 10.0 },
    MetricDef { name: "Centroid",           target: |t| t.spectral.centroid_hz,          actual: |a| a.spectral.centroid_hz,          unit: " Hz",   scale: 1000.0 },
    MetricDef { name: "Spectral flatness",  target: |t| t.spectral.flatness,             actual: |a| a.spectral.flatness,             unit: "",      scale: 0.5 },
    MetricDef { name: "Zero crossing rate", target: |t| t.spectral.zero_crossing_rate,   actual: |a| a.spectral.zero_crossing_rate,   unit: " /s",   scale: 500.0 },
    MetricDef { name: "Duration",           target: |t| t.temporal.duration_secs,        actual: |a| a.temporal.duration_secs,        unit: " s",    scale: 3.0 },
    MetricDef { name: "Speech rate",        target: |t| t.temporal.speech_rate,          actual: |a| a.temporal.speech_rate,          unit: " f/s",  scale: 50.0 },
    MetricDef { name: "Voiced ratio",       target: |t| t.voice.voiced_ratio,            actual: |a| a.pitch.voiced_ratio,            unit: "",      scale: 0.5 },
    MetricDef { name: "F0 stdDev",          target: |t| t.voice.f0_stddev_hz,            actual: |a| a.pitch.f0_stddev_hz,            unit: " Hz",   scale: 150.0 },
];

pub fn compute_gap(phrase: &str, actual: &AnalysisResult, tensor: &TargetTensor) -> GapResult {
    let rows: Vec<GapRow> = METRICS.iter().map(|m| {
        let t_val   = (m.target)(tensor);
        let a_val   = (m.actual)(actual);
        let gap_pct = ((a_val - t_val).abs() / m.scale * 100.0).min(100.0);
        let fmt     = |v: f32, unit: &str| format!("{:.2}{}", v, unit);
        GapRow {
            metric:  m.name.to_string(),
            target:  fmt(t_val, m.unit),
            actual:  fmt(a_val, m.unit),
            gap_pct: (gap_pct * 10.0).round() / 10.0,
            verdict: Verdict::for_gap(gap_pct),
        }
    }).collect();

    let mean_gap_pct = (rows.iter().map(|r| r.gap_pct).sum::<f32>() / rows.len() as f32 * 10.0).round() / 10.0;
    GapResult { phrase: phrase.to_string(), rows, mean_gap_pct }
}
