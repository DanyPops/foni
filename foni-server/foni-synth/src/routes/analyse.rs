use axum::{extract::State, http::StatusCode, Json};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use foni_analyse::{
    analyse as run_analyse, compute_gap, decode_wav, AnalysisResult, GapResult, TargetTensor,
};

#[derive(Deserialize)]
pub struct AnalyseRequest {
    /// Base64-encoded WAV bytes to analyse.
    pub audio_data: String,
    /// Optional reference WAV — if provided, also returns a GapResult.
    pub reference_data: Option<String>,
    /// Label for the reference (used in gap result description).
    pub reference_label: Option<String>,
    /// Phrase text (used in gap result).
    pub phrase: Option<String>,
}

#[derive(Serialize)]
pub struct AnalyseResponse {
    pub analysis: AnalysisResult,
    pub gap_result: Option<GapResult>,
}

pub async fn analyse(
    State(_state): State<AppState>,
    Json(req): Json<AnalyseRequest>,
) -> Result<Json<AnalyseResponse>, (StatusCode, String)> {
    let bytes = B64
        .decode(&req.audio_data)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("base64: {e}")))?;

    let wav =
        decode_wav(&bytes).map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, format!("wav: {e}")))?;

    // Decode reference WAV if provided
    let ref_bytes = req
        .reference_data
        .as_deref()
        .map(|b64| B64.decode(b64))
        .transpose()
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("reference base64: {e}")))?;

    let ref_wav = ref_bytes
        .as_deref()
        .map(decode_wav)
        .transpose()
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("reference wav: {e}"),
            )
        })?;

    let label = req
        .reference_label
        .unwrap_or_else(|| "reference".to_string());
    let phrase = req.phrase.unwrap_or_default();

    // CPU-bound — run on blocking thread pool
    let result = tokio::task::spawn_blocking(move || {
        let analysis = run_analyse(&wav.samples, wav.sample_rate);
        let gap_result = ref_wav.map(|r| {
            let ref_analysis = run_analyse(&r.samples, r.sample_rate);
            let tensor = TargetTensor::from_analysis(&ref_analysis, &label);
            compute_gap(&phrase, &analysis, &tensor)
        });
        (analysis, gap_result)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))?;

    Ok(Json(AnalyseResponse {
        analysis: result.0,
        gap_result: result.1,
    }))
}
