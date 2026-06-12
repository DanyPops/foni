/// POST /synthesize — text → TTS → DSP → WAV, with LRU cache.
///
/// TTS backend: Chatterbox Multilingual on Modal (DEPECHER_TTS_URL).
/// Zero-shot voice cloning with expression controls (excitement, assertiveness, warmth).
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{quality::dsp, state::AppState, wav};

use super::process_route::WireOpts;

const FISH_SPEECH_TIMEOUT_SECS: u64 = 120;

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
    /// Emotion intensity (0.25–2.0, default 0.5 = neutral)
    pub exaggeration: Option<f32>,
    /// Pace/guidance weight (0.0–1.0, default 0.5)
    pub cfg_weight: Option<f32>,
    /// Prosody randomness (0.05–5.0, default 0.8)
    pub temperature: Option<f32>,
    /// Stress annotation backend: "dict", "ruaccent", or "none" (default "none").
    #[serde(default)]
    pub stress_mode: Option<String>,
    /// Base64-encoded reference WAV for zero-shot voice cloning.
    /// Forwarded verbatim to the Chatterbox endpoint as `audio_prompt`.
    #[serde(default)]
    pub audio_prompt: Option<String>,
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
    for val in [req.exaggeration, req.cfg_weight, req.temperature]
        .into_iter()
        .flatten()
    {
        h.update(val.to_le_bytes());
    }
    if let Some(ref sm) = req.stress_mode {
        h.update(sm.as_bytes());
    }
    if let Some(ref ap) = req.audio_prompt {
        // Hash just the first 256 bytes of the base64 string — enough to
        // distinguish different reference clips without hashing the full payload.
        h.update(&ap.as_bytes()[..ap.len().min(256)]);
    }
    h.finalize().into()
}

fn tts_url() -> Option<String> {
    std::env::var("DEPECHER_TTS_URL")
        .or_else(|_| std::env::var("FISH_SPEECH_URL")) // legacy alias
        .ok()
}

fn tts_token() -> Option<String> {
    std::env::var("DEPECHER_TTS_TOKEN").ok()
}

async fn cloud_tts(text: &str, req: &SynthRequest) -> Result<Vec<u8>, String> {
    let t = std::time::Instant::now();
    let url = tts_url().ok_or("DEPECHER_TTS_URL not set")?;
    let mut body = serde_json::json!({"text": text, "language": req.voice});

    if let Some(v) = req.exaggeration {
        body["exaggeration"] = serde_json::json!(v);
    }
    if let Some(v) = req.cfg_weight {
        body["cfg_weight"] = serde_json::json!(v);
    }
    if let Some(v) = req.temperature {
        body["temperature"] = serde_json::json!(v);
    }
    if let Some(ref ap) = req.audio_prompt {
        body["audio_prompt"] = serde_json::json!(ap);
    }

    if let Some(token) = tts_token() {
        body["token"] = serde_json::Value::String(token);
    }

    let client = reqwest::Client::new();

    // First attempt
    let result = cloud_tts_request(&client, &url, &body).await;

    match result {
        Ok(bytes) => {
            tracing::info!(
                cloud_tts_ms = t.elapsed().as_millis() as u64,
                bytes = bytes.len(),
                "cloud_tts: done"
            );
            Ok(bytes)
        }
        Err(first_err) => {
            tracing::warn!(error = %first_err, "cloud_tts: first attempt failed, waking container");

            // Derive health URL from TTS URL
            let health_url = url
                .replace("-tts.modal.run", "-health.modal.run")
                .replace("/synthesize", "/health");
            let _ = client
                .get(&health_url)
                .timeout(std::time::Duration::from_secs(90))
                .send()
                .await;
            tracing::info!("cloud_tts: container warmed, retrying");

            // Retry
            let bytes = cloud_tts_request(&client, &url, &body).await?;
            tracing::info!(
                cloud_tts_ms = t.elapsed().as_millis() as u64,
                bytes = bytes.len(),
                "cloud_tts: done (after retry)"
            );
            Ok(bytes)
        }
    }
}

async fn cloud_tts_request(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<Vec<u8>, String> {
    let resp = client
        .post(url)
        .json(body)
        .timeout(std::time::Duration::from_secs(FISH_SPEECH_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("TTS request: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("TTS HTTP {}", resp.status()));
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("TTS body: {e}"))
}

/// Synthesize via cloud TTS (Chatterbox on Modal).
async fn synthesize_text(text: &str, req: &SynthRequest) -> Result<Vec<u8>, String> {
    cloud_tts(text, req).await
}

pub async fn synthesize(
    State(state): State<AppState>,
    Json(mut req): Json<SynthRequest>,
) -> Result<Response, (StatusCode, String)> {
    let t_start = std::time::Instant::now();

    // Apply per-model config (lang + reference audio) before cache key and synthesis.
    if let Some(ref model_name) = req.model.clone() {
        let model_dir = state.0.models_dir.join(model_name);
        if req.voice == default_voice() {
            if let Ok(lang) = std::fs::read_to_string(model_dir.join("lang")) {
                req.voice = lang.trim().to_string();
            }
        }
        if req.audio_prompt.is_none() {
            if let Ok(bytes) = std::fs::read(model_dir.join("reference.wav")) {
                use base64::Engine as _;
                let capped = match state.0.synth.max_reference_secs() {
                    Some(max) => crate::wav::cap_wav(&bytes, max),
                    None => bytes,
                };
                req.audio_prompt = Some(base64::engine::general_purpose::STANDARD.encode(&capped));
                tracing::info!(model = %model_name, "synthesize: using reference audio");
            }
        }
    }

    // Log metadata only — never log text content (OWASP A09 / data minimisation).
    let chars = req.text.chars().count();
    tracing::info!(chars, voice = %req.voice, dsp = req.dsp, "synthesize: start");

    let key = cache_key(&req);

    // Cache hit
    {
        let t = std::time::Instant::now();
        let mut cache = state.0.wav_cache.lock().await;
        if let Some(cached) = cache.get(&key) {
            tracing::info!(
                cache_ms = t.elapsed().as_millis() as u64,
                bytes = cached.len(),
                "synthesize: cache hit"
            );
            return wav_response(cached.clone());
        }
        tracing::debug!(
            cache_ms = t.elapsed().as_millis() as u64,
            "synthesize: cache miss"
        );
    }

    // TTS synthesis
    let t_tts = std::time::Instant::now();
    let text = {
        use crate::engine::stress::{make_annotator, StressMode};
        use std::str::FromStr;
        let mode = req
            .stress_mode
            .as_deref()
            .map(|s| StressMode::from_str(s).unwrap_or_default())
            .unwrap_or(StressMode::None);
        let ruaccent_url = std::env::var("DEPECHER_RUACCENT_URL")
            .unwrap_or_else(|_| "http://localhost:8765/annotate".into());
        let annotator = make_annotator(&mode, &ruaccent_url);
        let normalised = crate::engine::stream::normalise_numbers(&req.text);
        annotator.annotate(&normalised)
    };
    let backend = "cloud";
    let raw_tts = synthesize_text(&text, &req)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let tts_bytes = if backend == "cloud" {
        wav::roundtrip(&raw_tts, |samples, sr| {
            wav::noise_gate(samples, sr);
            wav::trim_tail(samples, sr);
        })
        .unwrap_or(raw_tts)
    } else {
        raw_tts
    };
    tracing::info!(
        tts_ms = t_tts.elapsed().as_millis() as u64,
        backend,
        bytes = tts_bytes.len(),
        "synthesize: tts done (tail trimmed)"
    );

    let t_dsp = std::time::Instant::now();
    let dsp_globally_enabled = state
        .0
        .dsp_enabled
        .load(std::sync::atomic::Ordering::Relaxed);
    let final_wav = if req.dsp && dsp_globally_enabled {
        // Base on the server’s configured dsp_defaults; request opts layer on top.
        let dsp_defaults = state.0.dsp_defaults.read().await.clone();
        let base = crate::quality::dsp::SmoothingOptions::from(&dsp_defaults);
        let (base_opts, _pad) = req.opts.into_smoothing_with_base(base);
        let controller_enabled = state
            .0
            .controller_enabled
            .load(std::sync::atomic::Ordering::Relaxed);
        let controller_cfg = state.0.controller_config.read().await.clone();
        let policy_arc = state.0.policy_engine.read().await.clone();
        tokio::task::spawn_blocking(move || {
            wav::roundtrip(&tts_bytes, |samples, sr| {
                let opts = if controller_enabled {
                    let analysis = depecher_analyse::analyse_fast(samples, sr);
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

    tracing::info!(
        dsp_ms = t_dsp.elapsed().as_millis() as u64,
        dsp = req.dsp && dsp_globally_enabled,
        bytes = final_wav.len(),
        "synthesize: dsp done"
    );

    {
        let mut cache = state.0.wav_cache.lock().await;
        cache.put(key, final_wav.clone());
    }

    let total_ms = t_start.elapsed().as_millis() as u64;
    tracing::info!(total_ms, "synthesize: complete");

    wav_response(final_wav)
}

fn wav_response(bytes: Vec<u8>) -> Result<Response, (StatusCode, String)> {
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .body(Body::from(bytes))
        .expect("infallible"))
}
