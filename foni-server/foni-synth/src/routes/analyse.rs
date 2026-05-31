use axum::{extract::State, http::StatusCode, Json};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};

use foni_analyse::{analyse as run_analyse, decode_wav, AnalysisResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AnalyseRequest {
    /// Base64-encoded WAV bytes to analyse.
    pub audio_data: String,
    /// Optional reference WAV — if provided, returns gap result too.
    pub reference_data: Option<String>,
}

#[derive(Serialize)]
pub struct AnalyseResponse {
    pub analysis: AnalysisResult,
    // TODO (FON-TSK-67): add gap_result: Option<GapResult> when reference_data provided
}

pub async fn analyse(
    State(_state): State<AppState>,
    Json(req): Json<AnalyseRequest>,
) -> Result<Json<AnalyseResponse>, (StatusCode, String)> {
    let bytes = B64.decode(&req.audio_data)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("base64: {e}")))?;

    let wav = decode_wav(&bytes)
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, format!("wav: {e}")))?;

    // CPU-bound — run on blocking thread pool so the async executor stays free.
    let analysis = tokio::task::spawn_blocking(move || run_analyse(&wav.samples, wav.sample_rate))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))?;

    Ok(Json(AnalyseResponse { analysis }))
}
