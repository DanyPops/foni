/// POST /process — apply Rust DSP chain to a WAV buffer.
///
/// Replaces the ffmpeg post-filter in pipeline/processors.ts SmoothingProcessor.
/// Input:  { audio_data: base64_wav }
/// Output: { audio_data: base64_wav }

use axum::{extract::State, http::StatusCode, Json};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};

use crate::{dsp, state::AppState, wav};

#[derive(Deserialize)]
pub struct ProcessRequest {
    /// Base64-encoded 16-bit PCM WAV to process.
    pub audio_data: String,
    /// Silence padding in seconds added before + after (for RVC edge context).
    /// Applied BEFORE the DSP chain. Default 0 (caller pads before RVC separately).
    #[serde(default)]
    pub pad_secs: f32,
}

#[derive(Serialize)]
pub struct ProcessResponse {
    pub audio_data: String,
}

pub async fn process(
    State(_state): State<AppState>,
    Json(req): Json<ProcessRequest>,
) -> Result<Json<ProcessResponse>, (StatusCode, String)> {
    let bytes = B64.decode(&req.audio_data)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("base64: {e}")))?;

    let pad = req.pad_secs;
    let opts = dsp::SmoothingOptions::default();

    let out = tokio::task::spawn_blocking(move || {
        wav::roundtrip(&bytes, |samples, sr| {
            // Optional silence padding (pre-RVC context — caller may have already done it)
            if pad > 0.0 {
                *samples = wav::pad_silence(samples, pad, sr);
            }
            // Full DSP chain
            *samples = dsp::apply(std::mem::take(samples), sr, &opts);
        })
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))?
    .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))?;

    Ok(Json(ProcessResponse { audio_data: B64.encode(&out) }))
}
