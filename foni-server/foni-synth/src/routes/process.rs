/// POST /process — apply Rust DSP chain to a WAV buffer.
use axum::{extract::State, http::StatusCode, Json};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};

use crate::{dsp, state::AppState, wav};

/// Subset of `SmoothingOptions` accepted over the wire (camelCase, matching TS).
/// Unknown fields are ignored. Omitted fields fall back to `Default`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WireOpts {
    pub rms_target_lufs: Option<f32>,
    pub compression_ratio: Option<f32>,
    pub compression_attack_ms: Option<f32>,
    pub compression_release_ms: Option<f32>,
    pub compression_threshold_db: Option<f32>,
    pub compression_makeup_db: Option<f32>,
    pub tilt_low_db: Option<f32>,
    pub tilt_high_db: Option<f32>,
    pub vibrato_freq: Option<f32>,
    pub vibrato_depth: Option<f32>,
    pub highpass_freq: Option<f32>,
    pub presence_db: Option<f32>,
    pub de_ess_db: Option<f32>,
    pub warmth_boost_db: Option<f32>,
    pub warmth_freq: Option<f32>,
    pub air_boost_db: Option<f32>,
    pub air_freq: Option<f32>,
    pub reverb_ms: Option<f32>,
    pub reverb_decay: Option<f32>,
    pub pad_secs: Option<f32>,
    pub fade_secs: Option<f32>,
}

impl WireOpts {
    pub(crate) fn into_smoothing(self) -> (dsp::SmoothingOptions, f32) {
        let mut o = dsp::SmoothingOptions::default();
        if let Some(v) = self.rms_target_lufs {
            o.rms_target_lufs = v;
        }
        if let Some(v) = self.compression_ratio {
            o.compression_ratio = v;
        }
        if let Some(v) = self.compression_attack_ms {
            o.compression_attack_ms = v;
        }
        if let Some(v) = self.compression_release_ms {
            o.compression_release_ms = v;
        }
        if let Some(v) = self.compression_threshold_db {
            o.compression_threshold_db = v;
        }
        if let Some(v) = self.compression_makeup_db {
            o.compression_makeup_db = v;
        }
        if let Some(v) = self.tilt_low_db {
            o.tilt_low_db = v;
        }
        if let Some(v) = self.tilt_high_db {
            o.tilt_high_db = v;
        }
        if let Some(v) = self.vibrato_freq {
            o.vibrato_freq = v;
        }
        if let Some(v) = self.vibrato_depth {
            o.vibrato_depth = v;
        }
        if let Some(v) = self.highpass_freq {
            o.highpass_freq = v;
        }
        if let Some(v) = self.presence_db {
            o.presence_db = v;
        }
        if let Some(v) = self.de_ess_db {
            o.de_ess_db = v;
        }
        if let Some(v) = self.warmth_boost_db {
            o.warmth_boost_db = v;
        }
        if let Some(v) = self.warmth_freq {
            o.warmth_freq = v;
        }
        if let Some(v) = self.air_boost_db {
            o.air_boost_db = v;
        }
        if let Some(v) = self.air_freq {
            o.air_freq = v;
        }
        if let Some(v) = self.reverb_ms {
            o.reverb_ms = v;
        }
        if let Some(v) = self.reverb_decay {
            o.reverb_decay = v;
        }
        if let Some(v) = self.fade_secs {
            o.fade_secs = v;
        }
        let pad = self.pad_secs.unwrap_or(0.0);
        (o, pad)
    }
}

#[derive(Deserialize)]
pub struct ProcessRequest {
    pub audio_data: String,
    /// DSP tuning overrides — any omitted field uses the server default.
    #[serde(default)]
    pub opts: WireOpts,
}

#[derive(Serialize)]
pub struct ProcessResponse {
    pub audio_data: String,
}

pub async fn process(
    State(_state): State<AppState>,
    Json(req): Json<ProcessRequest>,
) -> Result<Json<ProcessResponse>, (StatusCode, String)> {
    let bytes = B64
        .decode(&req.audio_data)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("base64: {e}")))?;

    let (opts, pad) = req.opts.into_smoothing();

    let out = tokio::task::spawn_blocking(move || {
        wav::roundtrip(&bytes, |samples, sr| {
            if pad > 0.0 {
                *samples = wav::pad_silence(samples, pad, sr);
            }
            *samples = dsp::apply(std::mem::take(samples), sr, &opts);
        })
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))?
    .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))?;

    Ok(Json(ProcessResponse {
        audio_data: B64.encode(&out),
    }))
}
