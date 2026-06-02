use crate::AnalysisResult;
use serde::{Deserialize, Serialize};

/// Desired-state acoustic fingerprint — loaded from baseline/target.json
/// or built from a single reference WAV via TargetTensor::from_analysis().
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetTensor {
    pub _description: String,
    pub _sources: Vec<String>,
    pub temporal: TargetTemporal,
    pub spectral: TargetSpectral,
    pub voice: TargetVoice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetTemporal {
    pub duration_secs: f32,
    pub speech_rate: f32,
    pub pause_count: f32,
    pub mean_pause_duration: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetSpectral {
    pub rms_db: f32,
    pub crest_factor: f32,
    pub brightness_hz: f32,
    pub tone_purity: f32,
    pub zero_crossing_rate: f32,
    #[serde(default)]
    pub bass_balance_db: f32,
    #[serde(default)]
    pub vocal_darkness_db_oct: f32,
    #[serde(default)]
    pub vocal_weight_hz: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetVoice {
    pub pitch_hz: f32,
    pub pitch_variation_hz: f32,
    pub voice_presence: f32,
    #[serde(default)]
    pub breathiness_db: f32,
}

impl TargetTensor {
    /// Build a tensor from a single reference analysis — no averaging.
    pub fn from_analysis(a: &AnalysisResult, label: &str) -> Self {
        TargetTensor {
            _description: label.to_string(),
            _sources: vec![label.to_string()],
            temporal: TargetTemporal {
                duration_secs: a.temporal.duration_secs,
                speech_rate: a.temporal.speech_rate,
                pause_count: a.temporal.pause_count as f32,
                mean_pause_duration: a.temporal.mean_pause_duration,
            },
            spectral: TargetSpectral {
                rms_db: a.loudness.rms_db,
                crest_factor: a.loudness.crest_factor,
                brightness_hz: a.spectral.brightness_hz,
                tone_purity: a.spectral.tone_purity,
                zero_crossing_rate: a.spectral.zero_crossing_rate,
                bass_balance_db: a.spectral.bass_balance_db,
                vocal_darkness_db_oct: a.spectral.vocal_darkness_db_oct,
                vocal_weight_hz: a.spectral.vocal_weight_hz,
            },
            voice: TargetVoice {
                pitch_hz: a.pitch.pitch_hz,
                pitch_variation_hz: a.pitch.pitch_variation_hz,
                voice_presence: a.pitch.voice_presence,
                breathiness_db: a.voice.breathiness_db,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum Verdict {
    #[serde(rename = "✅ close")]
    Close,
    #[serde(rename = "🟡 near")]
    Near,
    #[serde(rename = "🟠 far")]
    Far,
    #[serde(rename = "🔴 very far")]
    VeryFar,
}

impl Verdict {
    pub fn for_gap(pct: f32) -> Self {
        if pct < 15.0 {
            Verdict::Close
        } else if pct < 35.0 {
            Verdict::Near
        } else if pct < 60.0 {
            Verdict::Far
        } else {
            Verdict::VeryFar
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Close => "✅ close",
            Verdict::Near => "🟡 near",
            Verdict::Far => "🟠 far",
            Verdict::VeryFar => "🔴 very far",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GapRow {
    pub metric: String,
    pub target: String,
    pub actual: String,
    pub gap_pct: f32,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Serialize)]
pub struct GapResult {
    pub phrase: String,
    pub rows: Vec<GapRow>,
    pub mean_gap_pct: f32,
}

struct MetricDef {
    name: &'static str,
    target: fn(&TargetTensor) -> f32,
    actual: fn(&AnalysisResult) -> f32,
    unit: &'static str,
    scale: f32,
}

const METRICS: &[MetricDef] = &[
    MetricDef {
        name: "Loudness",
        target: |t| t.spectral.rms_db,
        actual: |a| a.loudness.rms_db,
        unit: " dBFS",
        scale: 10.0,
    },
    MetricDef {
        name: "Dynamic range",
        target: |t| t.spectral.crest_factor,
        actual: |a| a.loudness.crest_factor,
        unit: " dB",
        scale: 10.0,
    },
    MetricDef {
        name: "Brightness",
        target: |t| t.spectral.brightness_hz,
        actual: |a| a.spectral.brightness_hz,
        unit: " Hz",
        scale: 1000.0,
    },
    MetricDef {
        name: "Tone purity",
        target: |t| t.spectral.tone_purity,
        actual: |a| a.spectral.tone_purity,
        unit: "",
        scale: 0.5,
    },
    MetricDef {
        name: "Noise texture",
        target: |t| t.spectral.zero_crossing_rate,
        actual: |a| a.spectral.zero_crossing_rate,
        unit: " /s",
        scale: 500.0,
    },
    MetricDef {
        name: "Duration",
        target: |t| t.temporal.duration_secs,
        actual: |a| a.temporal.duration_secs,
        unit: " s",
        scale: 3.0,
    },
    MetricDef {
        name: "Speech rate",
        target: |t| t.temporal.speech_rate,
        actual: |a| a.temporal.speech_rate,
        unit: " f/s",
        scale: 50.0,
    },
    MetricDef {
        name: "Voice presence",
        target: |t| t.voice.voice_presence,
        actual: |a| a.pitch.voice_presence,
        unit: "",
        scale: 0.5,
    },
    MetricDef {
        name: "Pitch variation",
        target: |t| t.voice.pitch_variation_hz,
        actual: |a| a.pitch.pitch_variation_hz,
        unit: " Hz",
        scale: 150.0,
    },
    MetricDef {
        name: "Bass balance",
        target: |t| t.spectral.bass_balance_db,
        actual: |a| a.spectral.bass_balance_db,
        unit: " dB",
        scale: 15.0,
    },
    MetricDef {
        name: "Vocal darkness",
        target: |t| t.spectral.vocal_darkness_db_oct,
        actual: |a| a.spectral.vocal_darkness_db_oct,
        unit: " dB/oct",
        scale: 8.0,
    },
    MetricDef {
        name: "Vocal weight",
        target: |t| t.spectral.vocal_weight_hz,
        actual: |a| a.spectral.vocal_weight_hz,
        unit: " Hz",
        scale: 500.0,
    },
    MetricDef {
        name: "Breathiness",
        target: |t| t.voice.breathiness_db,
        actual: |a| a.voice.breathiness_db,
        unit: " dB",
        scale: 10.0,
    },
];

pub fn compute_gap(phrase: &str, actual: &AnalysisResult, tensor: &TargetTensor) -> GapResult {
    let rows: Vec<GapRow> = METRICS
        .iter()
        .map(|m| {
            let t_val = (m.target)(tensor);
            let a_val = (m.actual)(actual);
            let gap_pct = ((a_val - t_val).abs() / m.scale * 100.0).min(100.0);
            let fmt = |v: f32, unit: &str| format!("{:.2}{}", v, unit);
            GapRow {
                metric: m.name.to_string(),
                target: fmt(t_val, m.unit),
                actual: fmt(a_val, m.unit),
                gap_pct: (gap_pct * 10.0).round() / 10.0,
                verdict: Verdict::for_gap(gap_pct),
            }
        })
        .collect();

    let mean_gap_pct =
        (rows.iter().map(|r| r.gap_pct).sum::<f32>() / rows.len() as f32 * 10.0).round() / 10.0;
    GapResult {
        phrase: phrase.to_string(),
        rows,
        mean_gap_pct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnalysisResult;

    fn dummy_analysis() -> AnalysisResult {
        crate::analyse(&vec![0.0; 16000], 16000)
    }

    fn dummy_target() -> TargetTensor {
        TargetTensor::from_analysis(&dummy_analysis(), "test")
    }

    #[test]
    fn verdict_close_below_15() {
        assert!(matches!(Verdict::for_gap(0.0), Verdict::Close));
        assert!(matches!(Verdict::for_gap(14.9), Verdict::Close));
    }

    #[test]
    fn verdict_near_between_15_and_35() {
        assert!(matches!(Verdict::for_gap(15.0), Verdict::Near));
        assert!(matches!(Verdict::for_gap(34.9), Verdict::Near));
    }

    #[test]
    fn verdict_far_between_35_and_60() {
        assert!(matches!(Verdict::for_gap(35.0), Verdict::Far));
        assert!(matches!(Verdict::for_gap(59.9), Verdict::Far));
    }

    #[test]
    fn verdict_very_far_above_60() {
        assert!(matches!(Verdict::for_gap(60.0), Verdict::VeryFar));
        assert!(matches!(Verdict::for_gap(100.0), Verdict::VeryFar));
    }

    #[test]
    fn verdict_label_returns_emoji_string() {
        assert!(Verdict::Close.label().contains("✅"));
        assert!(Verdict::VeryFar.label().contains("🔴"));
    }

    #[test]
    fn from_analysis_copies_all_fields() {
        let a = dummy_analysis();
        let t = TargetTensor::from_analysis(&a, "sidorovich");
        assert_eq!(t._description, "sidorovich");
        assert_eq!(t.spectral.brightness_hz, a.spectral.brightness_hz);
        assert_eq!(t.voice.pitch_hz, a.pitch.pitch_hz);
        assert_eq!(t.temporal.duration_secs, a.temporal.duration_secs);
    }

    #[test]
    fn identical_analysis_has_zero_gap() {
        let a = dummy_analysis();
        let t = dummy_target();
        let result = compute_gap("test", &a, &t);
        assert!(
            result.mean_gap_pct < 10.0,
            "identical analysis should have near-zero gap, got {}%",
            result.mean_gap_pct
        );
    }

    #[test]
    fn gap_result_has_13_metrics() {
        let a = dummy_analysis();
        let t = dummy_target();
        let result = compute_gap("test", &a, &t);
        assert_eq!(result.rows.len(), 13);
    }

    #[test]
    fn gap_is_capped_at_100_percent() {
        let a = dummy_analysis();
        let mut t = dummy_target();
        t.spectral.brightness_hz = 99999.0;
        let result = compute_gap("test", &a, &t);
        for row in &result.rows {
            assert!(
                row.gap_pct <= 100.0,
                "{} gap exceeded 100%: {}",
                row.metric,
                row.gap_pct
            );
        }
    }

    #[test]
    fn gap_phrase_is_preserved() {
        let a = dummy_analysis();
        let t = dummy_target();
        let result = compute_gap("Привет, сталкер", &a, &t);
        assert_eq!(result.phrase, "Привет, сталкер");
    }

    #[test]
    fn target_tensor_serializes_to_json() {
        let t = dummy_target();
        let json = serde_json::to_string(&t);
        assert!(json.is_ok());
    }
}
