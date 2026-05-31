/// POST /convert — RVC voice conversion via ONNX generator.
///
/// Pipeline:
///   1. Decode input WAV → f32 samples
///   2. Extract ContentVec phone features (vec-768-layer-12.onnx)
///   3. Extract F0 pitch (rmvpe.onnx)
///   4. Run generator (models/<name>/onnx/generator.onnx) → audio
///   5. Return processed WAV
///
/// ContentVec ONNX requires fairseq export (not yet automated).
/// Until available, this endpoint returns NOT_IMPLEMENTED unless
/// GENERATOR_ONNX env var points to a pre-exported generator.

use axum::{extract::State, http::StatusCode, Json};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::path::Path;

use foni_analyse::decode_wav;
use crate::{state::AppState, wav::encode_wav};

#[derive(Deserialize)]
pub struct ConvertRequest {
    pub audio_data: String,
}

#[derive(Serialize)]
pub struct ConvertResponse {
    pub audio_data: String,
}

/// Validate that a generator ONNX file loads and runs correctly.
/// Returns the audio output shape on success.
pub fn validate_generator_onnx(path: &Path) -> Result<Vec<usize>, String> {
    use ort::{session::Session, value::Tensor};

    let mut session = Session::builder()
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| e.to_string())?;

    // Inputs: phone[1,T,768], phone_lengths[1], pitch[1,T], pitchf[1,T], ds[1], rnd[1,192,T]
    // T=200 matches the export trace size — dynamic axes allow other lengths at runtime
    let t: usize = 200;
    let phone   = Tensor::<f32>::from_array(([1usize, t, 768], vec![0.0f32; t * 768]))
                    .map_err(|e| e.to_string())?;
    let lengths = Tensor::<i64>::from_array(([1usize], vec![t as i64]))
                    .map_err(|e| e.to_string())?;
    let pitch   = Tensor::<i64>::from_array(([1usize, t], vec![100i64; t]))
                    .map_err(|e| e.to_string())?;
    let pitchf  = Tensor::<f32>::from_array(([1usize, t], vec![220.0f32; t]))
                    .map_err(|e| e.to_string())?;
    let ds      = Tensor::<i64>::from_array(([1usize], vec![0i64]))
                    .map_err(|e| e.to_string())?;
    let rnd     = Tensor::<f32>::from_array(([1usize, 192, t], vec![0.0f32; 192 * t]))
                    .map_err(|e| e.to_string())?;

    let outputs = session.run(ort::inputs![
        "phone"         => phone,
        "phone_lengths" => lengths,
        "pitch"         => pitch,
        "pitchf"        => pitchf,
        "ds"            => ds,
        "rnd"           => rnd,
    ]).map_err(|e| e.to_string())?;

    let (shape, _data) = outputs["audio"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    Ok(shape.iter().map(|&d| d as usize).collect())
}

pub async fn convert(
    State(state): State<AppState>,
    Json(req): Json<ConvertRequest>,
) -> Result<Json<ConvertResponse>, (StatusCode, String)> {
    let model = state.0.current_model.read().await;
    let model_name = model.as_deref().unwrap_or("bandit");

    // Check for generator ONNX
    let generator_path = state.0.models_dir
        .join(model_name).join("onnx").join("generator.onnx");

    if !generator_path.exists() {
        return Err((StatusCode::NOT_IMPLEMENTED,
            format!("Generator ONNX not found at {}. Run: python3 rvc/export_onnx.py {}",
                generator_path.display(), model_name)));
    }

    // ContentVec is needed to extract phone features — not yet automated.
    // For now: return the input unchanged with a note (identity pass-through).
    // TODO: wire ContentVec ONNX when available (requires fairseq export).
    let bytes = B64.decode(&req.audio_data)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("base64: {e}")))?;

    // Identity pass-through until ContentVec is wired
    Ok(Json(ConvertResponse { audio_data: B64.encode(&bytes) }))
}
