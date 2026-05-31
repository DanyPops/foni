/// POST /synthesize — full text → espeak → RVC → DSP → WAV, with LRU cache.
///
/// Request:
///   { "text": "...", "voice": "ru", "speed": 150,
///     "model": "bandit",  // optional
///     "speaker_id": 0,    // optional
///     "f0_up_key": 0,     // optional semitone shift
///     "dsp": true }       // optional, default true — apply /process DSP chain
///
/// Response: raw audio/wav bytes (same as /convert).
/// Cache key: SHA-256(text + model + voice + speed + speaker_id + f0_up_key + dsp).
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::process::Command;

use crate::{dsp, state::AppState, wav};

use super::convert::{resample_to_16k, run_contentvec, run_generator, run_rmvpe};
use super::process::WireOpts;

use foni_analyse::decode_wav;

// Re-export constants needed here (defined in convert.rs)
use crate::routes::convert::{CONTENTVEC_HOP, GENERATOR_SR};

#[derive(Deserialize)]
pub struct SynthRequest {
    pub text: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_speed")]
    pub speed: u32,
    pub model: Option<String>,
    #[serde(default)]
    pub speaker_id: i64,
    #[serde(default)]
    pub f0_up_key: i32,
    #[serde(default = "default_true")]
    pub dsp: bool,
    #[serde(default)]
    pub opts: WireOpts,
}

fn default_voice() -> String {
    "ru".into()
}
fn default_speed() -> u32 {
    150
}
fn default_true() -> bool {
    true
}

fn cache_key(req: &SynthRequest, model: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(req.text.as_bytes());
    h.update(b"\0");
    h.update(model.as_bytes());
    h.update(b"\0");
    h.update(req.voice.as_bytes());
    h.update(req.speed.to_le_bytes());
    h.update(req.speaker_id.to_le_bytes());
    h.update(req.f0_up_key.to_le_bytes());
    h.update([req.dsp as u8]);
    h.finalize().into()
}

fn espeak(text: &str, voice: &str, speed: u32) -> Result<Vec<u8>, String> {
    let tmp = tempfile::NamedTempFile::with_suffix(".wav").map_err(|e| e.to_string())?;
    let status = Command::new("espeak-ng")
        .args([
            "-v",
            voice,
            "-s",
            &speed.to_string(),
            "-w",
            tmp.path().to_str().unwrap(),
            text,
        ])
        .status()
        .map_err(|e| format!("espeak-ng: {e}"))?;
    if !status.success() {
        return Err("espeak-ng exited non-zero".into());
    }
    std::fs::read(tmp.path()).map_err(|e| e.to_string())
}

pub async fn synthesize(
    State(state): State<AppState>,
    Json(req): Json<SynthRequest>,
) -> Result<Response, (StatusCode, String)> {
    let model_name = {
        let guard = state.0.current_model.read().await;
        req.model
            .clone()
            .unwrap_or_else(|| guard.as_deref().unwrap_or("bandit").to_string())
    };

    let key = cache_key(&req, &model_name);

    // Cache hit — return immediately without touching sessions.
    {
        let mut cache = state.0.wav_cache.lock().await;
        if let Some(cached) = cache.get(&key) {
            tracing::debug!("cache hit for {:?}", &req.text[..req.text.len().min(30)]);
            return wav_response(cached.clone());
        }
    }

    // Espeak synthesis (CPU-bound, run in blocking thread).
    let text = req.text.clone();
    let voice = req.voice.clone();
    let speed = req.speed;
    let espeak_bytes = tokio::task::spawn_blocking(move || espeak(&text, &voice, speed))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Ensure ONNX sessions are loaded.
    crate::sessions::ensure(&state, &model_name)
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;

    // RVC conversion.
    let wav_in = decode_wav(&espeak_bytes)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("WAV decode: {e}")))?;
    let audio_16k = resample_to_16k(&wav_in.samples, wav_in.sample_rate);
    let t_prime = audio_16k.len() / CONTENTVEC_HOP;
    let t_phone = t_prime * 2;

    if t_phone == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Text too short to synthesize".into(),
        ));
    }

    let audio_out = {
        let mut pool_guard = state.0.sessions.lock().await;
        let pool = pool_guard.as_mut().expect("sessions loaded above");

        let phone = run_contentvec(&audio_16k, &mut pool.contentvec).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("ContentVec: {e}"),
            )
        })?;
        let (pitch, pitchf) = run_rmvpe(&audio_16k, t_phone, req.f0_up_key, &mut pool.rmvpe)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("RMVPE: {e}")))?;
        run_generator(
            phone,
            t_phone,
            pitch,
            pitchf,
            req.speaker_id,
            &mut pool.generator,
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Generator: {e}")))?
    };

    let rvc_wav = wav::encode_wav(&audio_out, GENERATOR_SR).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("WAV encode: {e}"),
        )
    })?;

    // Optional DSP chain (/process).
    let final_wav = if req.dsp {
        let (opts, _pad) = req.opts.into_smoothing();
        tokio::task::spawn_blocking(move || {
            wav::roundtrip(&rvc_wav, |samples, sr| {
                *samples = dsp::apply(std::mem::take(samples), sr, &opts);
            })
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
    } else {
        rvc_wav
    };

    // Store in cache.
    {
        let mut cache = state.0.wav_cache.lock().await;
        cache.put(key, final_wav.clone());
        tracing::debug!("cached synthesis ({} entries)", cache.len());
    }

    wav_response(final_wav)
}

fn wav_response(bytes: Vec<u8>) -> Result<Response, (StatusCode, String)> {
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .body(Body::from(bytes))
        .expect("infallible"))
}
