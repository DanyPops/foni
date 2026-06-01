//! Mixer TUI state — the single source of truth for the ratatui event loop.
use foni_client::{AnalyseResponse, FoniClient, WavData, WireOpts};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Panels ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Panel {
    #[default]
    Tracks,
    Params,
    Analyse,
}

impl Panel {
    pub fn next(self) -> Self {
        match self {
            Self::Tracks => Self::Params,
            Self::Params => Self::Analyse,
            Self::Analyse => Self::Tracks,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Tracks => "Tracks",
            Self::Params => "Params",
            Self::Analyse => "Analyse",
        }
    }
}

// ── Track ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Track {
    pub label: String,
    pub desc: String,
    pub path: PathBuf,
    pub opts: WireOpts,
    pub rating: Option<u8>,
    pub note: Option<String>,
    pub winner: bool,
    /// Cached acoustic analysis (populated by Analyse panel).
    pub analyse: Option<AnalyseResponse>,
}

impl Track {
    pub fn stars(&self) -> String {
        match self.rating {
            None => "·····".into(),
            Some(n) => format!("{}{}", "★".repeat(n as usize), "☆".repeat(5 - n as usize)),
        }
    }

    pub fn rms_db(&self) -> Option<f32> {
        self.analyse.as_ref().map(|a| a.analysis.loudness.rms_db)
    }
}

// ── Param knobs ───────────────────────────────────────────────────────────────

pub struct ParamDef {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub format: fn(f32) -> String,
}

pub static PARAMS: &[ParamDef] = &[
    ParamDef {
        key: "vibratoDepth",
        label: "Vibrato depth",
        min: 0.0,
        max: 0.02,
        step: 0.001,
        format: |v| format!("{v:.3}"),
    },
    ParamDef {
        key: "vibratoFreq",
        label: "Vibrato Hz",
        min: 1.0,
        max: 12.0,
        step: 0.5,
        format: |v| format!("{v:.1} Hz"),
    },
    ParamDef {
        key: "compressionRatio",
        label: "Comp ratio",
        min: 1.0,
        max: 10.0,
        step: 0.5,
        format: |v| format!("{v:.1}:1"),
    },
    ParamDef {
        key: "compressionMakeupDb",
        label: "Comp makeup",
        min: 0.0,
        max: 12.0,
        step: 0.5,
        format: |v| format!("+{v:.1} dB"),
    },
    ParamDef {
        key: "tiltLowDb",
        label: "Low shelf",
        min: -6.0,
        max: 20.0,
        step: 1.0,
        format: |v| format!("{v:+.0} dB"),
    },
    ParamDef {
        key: "tiltHighDb",
        label: "High shelf",
        min: -20.0,
        max: 6.0,
        step: 1.0,
        format: |v| format!("{v:+.0} dB"),
    },
    ParamDef {
        key: "presenceDb",
        label: "Presence",
        min: -6.0,
        max: 6.0,
        step: 0.5,
        format: |v| format!("{v:+.1} dB"),
    },
    ParamDef {
        key: "deEssDb",
        label: "De-esser",
        min: 0.0,
        max: 12.0,
        step: 0.5,
        format: |v| format!("-{v:.1} dB"),
    },
    ParamDef {
        key: "reverbMs",
        label: "Reverb delay",
        min: 0.0,
        max: 80.0,
        step: 4.0,
        format: |v| format!("{v:.0} ms"),
    },
    ParamDef {
        key: "reverbDecay",
        label: "Reverb decay",
        min: 0.0,
        max: 0.5,
        step: 0.02,
        format: |v| format!("{v:.2}"),
    },
    ParamDef {
        key: "rmsTargetLufs",
        label: "Loudness target",
        min: -20.0,
        max: -4.0,
        step: 0.5,
        format: |v| format!("{v:.1} LUFS"),
    },
];

// ── Input mode ────────────────────────────────────────────────────────────────

#[derive(Default, PartialEq, Eq)]
pub enum InputMode {
    #[default]
    Normal,
    /// Typing a rating (1-5) after pressing a digit key.
    Rating { track_idx: usize, score: u8 },
    /// Typing a note for a track.
    Note { track_idx: usize, buf: String },
    /// Typing a name for a new render.
    RenderName { buf: String },
}

// ── Session file ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
pub struct SessionRecord {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub opts: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_pct: Option<f32>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Session {
    pub phrase: String,
    pub model: String,
    pub saved_at: String,
    pub tracks: Vec<SessionRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ai_suggestions: Vec<AiSuggestion>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AiSuggestion {
    pub label: String,
    pub rationale: String,
    pub opts: serde_json::Value,
}

// ── App ───────────────────────────────────────────────────────────────────────

pub struct MixerApp {
    pub client: FoniClient,
    pub phrase: String,
    pub model: String,
    pub tracks: Vec<Track>,
    pub selected: usize,
    pub panel: Panel,
    pub param_sel: usize,
    pub scratch: serde_json::Value,
    pub last_played: Option<usize>,
    pub input_mode: InputMode,
    /// Message shown in footer (cleared after next render).
    pub status_msg: Option<String>,
    /// True while a render is in progress (shows spinner).
    pub rendering: bool,
    /// True while analyse is loading.
    pub analysing: bool,
    pub reference: Option<std::path::PathBuf>,
    pub playing: bool,
    pub quit: bool,
}

impl MixerApp {
    pub fn new(client: FoniClient, phrase: String, model: String, tracks: Vec<Track>) -> Self {
        let scratch = serde_json::json!({
            "vibratoDepth": 0.0,
            "compressionRatio": 4.0,
            "compressionMakeupDb": 5.0,
            "tiltLowDb": 10.0,
            "tiltHighDb": -8.0,
            "reverbMs": 8.0,
            "reverbDecay": 0.04,
            "rmsTargetLufs": -8.0
        });
        MixerApp {
            client,
            phrase,
            model,
            tracks,
            selected: 0,
            panel: Panel::default(),
            param_sel: 0,
            scratch,
            last_played: None,
            input_mode: InputMode::Normal,
            status_msg: None,
            rendering: false,
            analysing: false,
            reference: None,
            playing: false,
            quit: false,
        }
    }

    pub fn selected_track(&self) -> Option<&Track> {
        self.tracks.get(self.selected)
    }

    pub fn scratch_get(&self, key: &str) -> f32 {
        self.scratch
            .get(key)
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32
    }

    pub fn scratch_set(&mut self, key: &str, val: f32) {
        self.scratch[key] = serde_json::Value::from(val as f64);
    }

    pub fn scratch_as_wire_opts(&self) -> WireOpts {
        serde_json::from_value(self.scratch.clone()).unwrap_or_default()
    }

    pub fn fork_from(&mut self, idx: usize) {
        if let Some(t) = self.tracks.get(idx) {
            self.scratch = serde_json::to_value(&t.opts).unwrap_or_default();
            self.status_msg = Some(format!("forked from {}", t.label));
        }
    }

    pub fn set_winner(&mut self, idx: usize) {
        for t in self.tracks.iter_mut() {
            t.winner = false;
        }
        if let Some(t) = self.tracks.get_mut(idx) {
            t.winner = true;
            self.status_msg = Some(format!("✪  winner: {}", t.label));
        }
    }

    pub fn set_rating(&mut self, idx: usize, score: u8, note: Option<String>) {
        if let Some(t) = self.tracks.get_mut(idx) {
            t.rating = Some(score);
            if let Some(n) = note {
                t.note = Some(n);
            }
        }
    }

    pub fn save_session(&self) {
        let session = Session {
            phrase: self.phrase.clone(),
            model: self.model.clone(),
            saved_at: {
                use std::time::{SystemTime, UNIX_EPOCH};
                let s = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                format!("{}Z", s) // good enough for QA logs
            },
            tracks: self
                .tracks
                .iter()
                .map(|t| SessionRecord {
                    label: t.label.clone(),
                    rating: t.rating,
                    winner: if t.winner { Some(true) } else { None },
                    note: t.note.clone(),
                    opts: serde_json::to_value(&t.opts).unwrap_or_default(),
                    gap_pct: t
                        .analyse
                        .as_ref()
                        .and_then(|a| a.gap_result.as_ref())
                        .map(|g| g.mean_gap_pct),
                })
                .collect(),
            ai_suggestions: Vec::new(),
        };
        let path = session_path();
        if let Ok(json) = serde_json::to_string_pretty(&session) {
            std::fs::write(path, json).ok();
        }
    }
}

pub fn session_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
        });
    let dir = base.join("foni");
    std::fs::create_dir_all(&dir).ok();
    dir.join("mixer-session.json")
}

pub fn load_session() -> Option<Session> {
    let data = std::fs::read_to_string(session_path()).ok()?;
    serde_json::from_str(&data).ok()
}
