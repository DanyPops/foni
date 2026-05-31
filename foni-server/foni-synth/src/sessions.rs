/// Lazy ONNX session loader.
///
/// Called the first time `/convert` runs, or explicitly from `POST /models/:name`.
/// All three models are kept alive in `AppState::sessions` for the server lifetime.
use std::path::{Path, PathBuf};

use ort::session::Session;

use crate::state::{AppState, OnnxPool};

fn load(path: &Path) -> Result<Session, String> {
    Session::builder()
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| e.to_string())
}

fn pretrained(models_dir: &Path, filename: &str) -> Option<PathBuf> {
    let p = models_dir.join("pretrained").join(filename);
    if p.exists() {
        return Some(p);
    }
    for base in &["../../rvc/models/pretrained", "../rvc/models/pretrained"] {
        let q = PathBuf::from(base).join(filename);
        if q.exists() {
            return Some(q);
        }
    }
    None
}

/// Ensure sessions are loaded for `model_name`, loading from disk if needed.
/// Re-loads only if the model name changed.
/// Returns `Err` with a human-readable message if any model file is missing.
pub async fn ensure(state: &AppState, model_name: &str) -> Result<(), String> {
    let mut guard = state.0.sessions.lock().await;

    if let Some(pool) = &*guard {
        if pool.model_name == model_name {
            return Ok(());
        }
    }

    let dir = &state.0.models_dir;

    let cv_path = pretrained(dir, "contentvec-768-l12.onnx")
        .ok_or("ContentVec ONNX not found — run: python3 rvc/export_contentvec_onnx.py")?;
    let rmvpe_path = pretrained(dir, "rmvpe.onnx")
        .ok_or("RMVPE ONNX not found — download from HuggingFace lj1995/VoiceConversionWebUI")?;
    let gen_path = dir.join(model_name).join("onnx").join("generator.onnx");
    if !gen_path.exists() {
        return Err(format!(
            "Generator ONNX not found at {} — run: python3 rvc/export_onnx.py {model_name}",
            gen_path.display()
        ));
    }

    tracing::info!("loading ONNX sessions for model '{model_name}'");
    let contentvec = load(&cv_path)?;
    let rmvpe = load(&rmvpe_path)?;
    let generator = load(&gen_path)?;
    tracing::info!("ONNX sessions ready");

    *guard = Some(OnnxPool {
        contentvec,
        rmvpe,
        generator,
        model_name: model_name.to_string(),
    });
    Ok(())
}
