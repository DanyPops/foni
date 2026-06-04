/// POST /synthesize — text → TTS → DSP → WAV, with LRU cache.
///
/// TTS backend priority:
///   1. Fish Speech API server (if FISH_SPEECH_URL is set and reachable)
///   2. espeak-ng fallback (always available)
///
/// Fish Speech provides voice identity via zero-shot cloning or fine-tuned model.
/// espeak provides robotic but instant synthesis when Fish Speech is unavailable.
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

use crate::{quality::dsp, state::AppState, wav};

use super::process_route::WireOpts;

const FISH_SPEECH_TIMEOUT_SECS: u64 = 30;

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
    #[serde(default = "default_true")]
    pub prosody: bool,
    pub rate_pct: Option<i32>,
    pub range: Option<String>,
    #[serde(default)]
    pub opts: WireOpts,
}

fn default_voice() -> String {
    "ru".into()
}
fn default_speed() -> u32 {
    135
}
fn default_true() -> bool {
    true
}

fn cache_key(req: &SynthRequest) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(req.text.as_bytes());
    h.update(b"\0");
    h.update(req.voice.as_bytes());
    h.update(req.speed.to_le_bytes());
    h.update([req.dsp as u8, req.prosody as u8]);
    if let Some(r) = req.rate_pct {
        h.update(r.to_le_bytes());
    }
    if let Some(ref rng) = req.range {
        h.update(rng.as_bytes());
    }
    let o = &req.opts;
    for val in [
        o.rms_target_lufs,
        o.compression_ratio,
        o.compression_attack_ms,
        o.compression_release_ms,
        o.compression_threshold_db,
        o.compression_makeup_db,
        o.tilt_low_db,
        o.tilt_high_db,
        o.de_harsh_db,
        o.de_harsh_freq,
        o.de_harsh_q,
    ]
    .into_iter()
    .flatten()
    {
        h.update(val.to_le_bytes());
    }
    h.finalize().into()
}

fn tts_url() -> Option<String> {
    std::env::var("FISH_SPEECH_URL").ok()
}

fn tts_token() -> Option<String> {
    std::env::var("FONI_TTS_TOKEN").ok()
}

async fn cloud_tts(text: &str, _reference_id: Option<&str>) -> Result<Vec<u8>, String> {
    let url = tts_url().ok_or("FISH_SPEECH_URL not set")?;
    let mut body = serde_json::json!({"text": text, "language": "ru"});

    if let Some(token) = tts_token() {
        body["token"] = serde_json::Value::String(token);
    }

    let client = reqwest::Client::new();
    let req = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(FISH_SPEECH_TIMEOUT_SECS));

    let resp = req.send().await.map_err(|e| format!("TTS request: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("TTS HTTP {}", resp.status()));
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("TTS body: {e}"))
}

fn espeak(text: &str, voice: &str, speed: u32) -> Result<Vec<u8>, String> {
    let tmp = tempfile::NamedTempFile::new().map_err(|e| format!("tmpfile: {e}"))?;
    let mut cmd = Command::new("espeak-ng");
    cmd.args([
        voice,
        "-s",
        &speed.to_string(),
        "-w",
        tmp.path()
            .to_str()
            .expect("infallible: tempfile path is valid UTF-8"),
    ]);
    if req_is_ssml(text) {
        cmd.arg("-m");
    }
    cmd.arg(text);
    let status = cmd.status().map_err(|e| format!("espeak-ng: {e}"))?;
    if !status.success() {
        return Err("espeak-ng exited non-zero".into());
    }
    std::fs::read(tmp.path()).map_err(|e| format!("read espeak output: {e}"))
}

fn req_is_ssml(text: &str) -> bool {
    text.contains("<speak") || text.contains("<break")
}

/// Try Fish Speech first, fall back to espeak.
async fn synthesize_text(
    text: &str,
    voice: &str,
    speed: u32,
    reference_id: Option<&str>,
) -> Result<Vec<u8>, String> {
    if tts_url().is_some() {
        match cloud_tts(text, reference_id).await {
            Ok(wav) => return Ok(wav),
            Err(e) => tracing::warn!("Fish Speech failed, falling back to espeak: {e}"),
        }
    }
    let voice = voice.to_string();
    let text = text.to_string();
    tokio::task::spawn_blocking(move || espeak(&text, &voice, speed))
        .await
        .map_err(|e| e.to_string())?
}

pub async fn synthesize(
    State(state): State<AppState>,
    Json(req): Json<SynthRequest>,
) -> Result<Response, (StatusCode, String)> {
    let t_start = std::time::Instant::now();

    let key = cache_key(&req);

    // Cache hit
    {
        let mut cache = state.0.wav_cache.lock().await;
        if let Some(cached) = cache.get(&key) {
            tracing::debug!("cache hit for {:?}", &req.text[..req.text.len().min(30)]);
            return wav_response(cached.clone());
        }
    }

    // TTS synthesis: Fish Speech → espeak fallback
    let reference_id = req.model.clone();
    let voice = req.voice.clone();
    let speed = req.speed;
    let text = req.text.clone();
    let tts_bytes = synthesize_text(&text, &voice, speed, reference_id.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // DSP chain with reactive controller
    let dsp_globally_enabled = state
        .0
        .dsp_enabled
        .load(std::sync::atomic::Ordering::Relaxed);
    let final_wav = if req.dsp && dsp_globally_enabled {
        let (base_opts, _pad) = req.opts.into_smoothing();
        let controller_enabled = state
            .0
            .controller_enabled
            .load(std::sync::atomic::Ordering::Relaxed);
        let controller_cfg = state.0.controller_config.read().await.clone();
        let policy_arc = state.0.policy_engine.read().await.clone();
        tokio::task::spawn_blocking(move || {
            wav::roundtrip(&tts_bytes, |samples, sr| {
                let opts = if controller_enabled {
                    let analysis = foni_analyse::analyse_fast(samples, sr);
                    if let Some(ref policy) = policy_arc {
                        if let Some((corrected, _snap)) =
                            policy.evaluate(&analysis, &base_opts, &controller_cfg)
                        {
                            corrected
                        } else {
                            let (corrected, _) =
                                dsp::controller::correct(&analysis, &base_opts, &controller_cfg);
                            corrected
                        }
                    } else {
                        let (corrected, _) =
                            dsp::controller::correct(&analysis, &base_opts, &controller_cfg);
                        corrected
                    }
                } else {
                    base_opts
                };
                *samples = dsp::apply(std::mem::take(samples), sr, &opts);
            })
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
    } else {
        tts_bytes
    };

    // Cache store
    {
        let mut cache = state.0.wav_cache.lock().await;
        cache.put(key, final_wav.clone());
    }

    let ms = t_start.elapsed().as_millis() as u64;
    tracing::debug!(synthesis_ms = ms);

    wav_response(final_wav)
}

fn wav_response(bytes: Vec<u8>) -> Result<Response, (StatusCode, String)> {
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .body(Body::from(bytes))
        .expect("infallible"))
}
