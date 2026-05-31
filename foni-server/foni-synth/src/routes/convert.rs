use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ConvertRequest {
    pub audio_data: String, // base64 WAV
}

#[derive(Serialize)]
pub struct ConvertResponse {
    pub audio_data: String, // base64 WAV
}

pub async fn convert(
    State(_state): State<AppState>,
    Json(_req): Json<ConvertRequest>,
) -> Result<Json<ConvertResponse>, (StatusCode, String)> {
    // TODO (FON-TSK-12): decode WAV, run RVC ONNX inference, return processed WAV
    Err((StatusCode::NOT_IMPLEMENTED, "RVC inference not yet implemented — use Python facade".to_string()))
}
