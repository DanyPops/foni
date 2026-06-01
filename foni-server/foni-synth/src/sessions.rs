//! ONNX session pool loader.
//!
//! `ensure()` fills every slot in the pool with loaded sessions for `model_name`.
//! Already-loaded slots with the same model are skipped.
use std::path::{Path, PathBuf};

use ort::session::Session;

use crate::state::{AppState, OnnxSession};

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

/// Fill every pool slot with sessions for `model_name`.
/// Slots already loaded for the same model are left untouched.
pub async fn ensure(state: &AppState, model_name: &str) -> Result<(), String> {
    if state.0.sessions.all_loaded(model_name).await {
        return Ok(());
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

    let pool = &state.0.sessions;
    let size = pool.size;
    tracing::info!("loading ONNX sessions for model '{model_name}' (pool_size={size})");

    for (i, slot) in pool.slots.iter().enumerate() {
        let mut guard = slot.lock().await;
        if guard
            .as_ref()
            .map(|s| s.model_name == model_name)
            .unwrap_or(false)
        {
            continue;
        }
        tracing::debug!("loading slot {i}/{size}");
        let contentvec = load(&cv_path)?;
        let rmvpe = load(&rmvpe_path)?;
        let generator = load(&gen_path)?;
        *guard = Some(OnnxSession {
            contentvec,
            rmvpe,
            generator,
            model_name: model_name.to_string(),
        });
    }

    tracing::info!("ONNX sessions ready (pool_size={size})");
    Ok(())
}
