//! Request/response types mirroring foni-synth DTOs.
//!
//! These are intentionally duplicated (not re-exported from foni-synth) so that
//! foni-client has zero dependency on axum, ort, or any server-side crate.
//! A round-trip JSON compatibility test in foni-synth/tests/ guards the contract.
use serde::{Deserialize, Serialize};

// ── WavData ───────────────────────────────────────────────────────────────────

/// Opaque WAV byte buffer — the single audio type used across all endpoints.
///
/// Hides the raw-bytes vs base64-JSON encoding split:
/// - /synthesize and /convert return `audio/wav` bodies → `WavData` directly
/// - /process and /analyse use `{ "audio_data": "<base64>" }` JSON
#[derive(Clone, Default)]
pub struct WavData(pub Vec<u8>);

// ── POST /synthesize ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthRequest {
    pub text: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_speed")]
    pub speed: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub speaker_id: i64,
    #[serde(default)]
    pub f0_up_key: i32,
    #[serde(default = "default_true")]
    pub dsp: bool,
    #[serde(default = "default_true")]
    pub prosody: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_pct: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(default)]
    pub opts: WireOpts,
}

impl SynthRequest {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            voice: default_voice(),
            speed: default_speed(),
            model: None,
            speaker_id: 0,
            f0_up_key: 0,
            dsp: true,
            prosody: true,
            rate_pct: None,
            range: None,
            opts: WireOpts::default(),
        }
    }
}

// ── POST /convert ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConvertRequest {
    pub audio_data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub speaker_id: i64,
    #[serde(default)]
    pub f0_up_key: i32,
}

// ── POST /process ─────────────────────────────────────────────────────────────

/// DSP tuning overrides — any omitted field uses the server default.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WireOpts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rms_target_lufs: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_ratio: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_attack_ms: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_release_ms: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_threshold_db: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_makeup_db: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tilt_low_db: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tilt_high_db: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato_freq: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato_depth: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highpass_freq: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_db: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub de_ess_db: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmth_boost_db: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmth_freq: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub air_boost_db: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub air_freq: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverb_ms: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverb_decay: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pad_secs: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_secs: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessResponse {
    pub audio_data: String,
}

// ── POST /analyse ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalyseRequest {
    pub audio_data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_label: Option<String>,
}

/// Mirrors foni_analyse::AnalysisResult — only the fields fonictl actually uses.
/// Full type lives in foni-analyse; we mirror a minimal subset here to avoid
/// pulling in the full analysis crate as a client dependency.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AnalyseResponse {
    pub analysis: AnalysisResult,
    pub gap_result: Option<GapResult>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AnalysisResult {
    pub temporal: TemporalResult,
    pub loudness: LoudnessResult,
    pub spectral: SpectralResult,
    pub pitch: PitchResult,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TemporalResult {
    pub duration_secs: f32,
    pub speech_rate: f32,
    pub pause_count: usize,
    pub mean_pause_duration: f32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LoudnessResult {
    pub rms_db: f32,
    pub crest_factor: f32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SpectralResult {
    pub brightness_hz: f32,
    pub tone_purity: f32,
    pub zero_crossing_rate: f32,
    pub bass_balance_db: f32,
    pub vocal_darkness_db_oct: f32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PitchResult {
    pub pitch_hz: f32,
    pub pitch_variation_hz: f32,
    pub voice_presence: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GapResult {
    pub mean_gap_pct: f32,
    pub rows: Vec<GapRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GapRow {
    pub metric: String,
    pub target: f32,
    pub actual: f32,
    pub gap_pct: f32,
    pub verdict: String,
}

// ── POST /breath ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BreathRequest {
    pub duration_ms: u32,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BreathResponse {
    pub audio_data: String,
}

// ── GET /models ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ModelsResponse {
    pub models: Vec<String>,
    pub onnx_ready: Vec<String>,
}

// ── GET /params ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RvcParams {
    pub f0up_key: i32,
    pub index_rate: f32,
    pub filter_radius: u32,
    pub rms_mix_rate: f32,
    pub protect: f32,
    pub f0method: String,
}

impl Default for RvcParams {
    fn default() -> Self {
        Self {
            f0up_key: -2,
            index_rate: 0.77,
            filter_radius: 5,
            rms_mix_rate: 0.45,
            protect: 0.33,
            f0method: "rmvpe".into(),
        }
    }
}

// ── serde defaults ────────────────────────────────────────────────────────────

fn default_voice() -> String {
    "ru".into()
}
fn default_speed() -> u32 {
    150
}
fn default_true() -> bool {
    true
}
