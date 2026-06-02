/// POST /synthesize — full text → SSML → espeak → RVC → DSP → WAV, with LRU cache.
///
/// Request:
///   { "text": "...", "voice": "ru", "speed": 150,
///     "model": "bandit",     // optional
///     "speaker_id": 0,       // optional
///     "f0_up_key": 0,        // optional semitone shift
///     "dsp": true,           // optional, default true
///     "prosody": true,       // optional, default true — per-sentence SSML prosody
///     "rate_pct": 100,       // optional — override rate (overrides per-sentence jitter)
///     "range": "medium" }    // optional — override range
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

use crate::{dsp, ssml, state::AppState, wav};

use super::convert::{
    apply_rms_mix, highpass_48hz, resample_to_16k, run_contentvec, run_generator, run_rmvpe,
};
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
    /// Apply per-sentence SSML prosody variation (rate / pitch / range).
    #[serde(default = "default_true")]
    pub prosody: bool,
    /// Override global rate % (100 = normal). Ignored when prosody=true (per-sentence wins).
    pub rate_pct: Option<i32>,
    /// Override global range: "x-high" | "high" | "medium" | "low" | "x-low".
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
    h.update([req.dsp as u8, req.prosody as u8]);
    if let Some(r) = req.rate_pct {
        h.update(r.to_le_bytes());
    }
    if let Some(ref rng) = req.range {
        h.update(rng.as_bytes());
    }
    // DSP opts — hash every field so different configs never collide.
    let o = &req.opts;
    for v in [
        o.rms_target_lufs,
        o.compression_ratio,
        o.compression_attack_ms,
        o.compression_release_ms,
        o.compression_threshold_db,
        o.compression_makeup_db,
        o.tilt_low_db,
        o.tilt_high_db,
        o.vibrato_freq,
        o.vibrato_depth,
        o.highpass_freq,
        o.presence_db,
        o.de_ess_db,
        o.warmth_boost_db,
        o.warmth_freq,
        o.air_boost_db,
        o.air_freq,
        o.reverb_ms,
        o.reverb_decay,
        o.pad_secs,
        o.fade_secs,
    ] {
        h.update(v.map(f32::to_le_bytes).unwrap_or([0xff; 4]));
    }
    h.finalize().into()
}

/// Prepare the text for espeak: apply SSML annotation if prosody is enabled.
/// Returns `(ssml_text, use_markup_flag)`.
fn prepare_text(req: &SynthRequest) -> (String, bool) {
    if !req.prosody {
        return (req.text.clone(), false);
    }

    // Per-sentence prosody variation (deterministic, mirrors prosody.ts).
    let annotated = ssml::annotate_with_prosody(&req.text);

    // Optional caller overrides wrap the whole block.
    let body = if req.rate_pct.is_some() || req.range.is_some() {
        let rate = req.rate_pct.unwrap_or(100);
        let range = req.range.as_deref().unwrap_or("medium");
        // Strip outer <speak> tags, re-wrap with prosody override.
        let inner = annotated
            .strip_prefix("<speak>")
            .unwrap_or(&annotated)
            .strip_suffix("</speak>")
            .unwrap_or(&annotated);
        format!(r#"<speak><prosody rate="{rate}%" range="{range}">{inner}</prosody></speak>"#)
    } else {
        annotated
    };

    (body, true)
}

fn espeak(text: &str, voice: &str, speed: u32, markup: bool) -> Result<Vec<u8>, String> {
    let tmp = tempfile::NamedTempFile::with_suffix(".wav").map_err(|e| e.to_string())?;
    let mut cmd = Command::new("espeak-ng");
    cmd.args([
        "-v",
        voice,
        "-s",
        &speed.to_string(),
        "-w",
        tmp.path()
            .to_str()
            .expect("infallible: tempfile path is valid UTF-8"),
    ]);
    if markup {
        cmd.arg("-m");
    }
    cmd.arg(text);
    let status = cmd.status().map_err(|e| format!("espeak-ng: {e}"))?;
    if !status.success() {
        return Err("espeak-ng exited non-zero".into());
    }
    std::fs::read(tmp.path()).map_err(|e| e.to_string())
}

pub async fn synthesize(
    State(state): State<AppState>,
    Json(req): Json<SynthRequest>,
) -> Result<Response, (StatusCode, String)> {
    let t_start = std::time::Instant::now();
    let model_name = {
        let guard = state.0.current_model.read().await;
        req.model
            .clone()
            .unwrap_or_else(|| guard.as_deref().unwrap_or("sidorovich").to_string())
    };

    let key = cache_key(&req, &model_name);

    // Cache hit — return immediately without touching sessions.
    {
        let mut cache = state.0.wav_cache.lock().await;
        if let Some(cached) = cache.get(&key) {
            tracing::debug!("cache hit for {:?}", &req.text[..req.text.len().min(30)]);
            use std::sync::atomic::Ordering;
            state.0.sessions.cache_hits.fetch_add(1, Ordering::Relaxed);
            return wav_response(cached.clone());
        }
    }

    // Prepare text (SSML annotation when prosody=true).
    let (synth_text, markup) = prepare_text(&req);

    // Espeak synthesis (CPU-bound, run in blocking thread).
    let voice = req.voice.clone();
    let speed = req.speed;
    let espeak_bytes =
        tokio::task::spawn_blocking(move || espeak(&synth_text, &voice, speed, markup))
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
    let mut audio_16k = resample_to_16k(&wav_in.samples, wav_in.sample_rate);
    highpass_48hz(&mut audio_16k);
    let t_prime = audio_16k.len() / CONTENTVEC_HOP;
    let t_phone = t_prime * 2;

    if t_phone == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Text too short to synthesize".into(),
        ));
    }

    let mut audio_out = {
        let mut pool_guard = state.0.sessions.acquire().await;
        let pool = pool_guard.session.as_mut().expect("sessions loaded above");

        let vi = state.0.voice_index.read().await;
        let params = state.0.params.read().await;
        let phone = run_contentvec(
            &audio_16k,
            &mut pool.contentvec,
            vi.as_ref(),
            params.index_rate,
        )
        .map_err(|e| {
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

    apply_rms_mix(&audio_16k, &mut audio_out, GENERATOR_SR, 0.45);
    let rvc_wav = wav::encode_wav(&audio_out, GENERATOR_SR).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("WAV encode: {e}"),
        )
    })?;

    // DSP chain — with reactive controller if enabled.
    // The controller measures the raw RVC output and corrects DSP params to match
    // the Sidorovich studio target. Falls back to defaults if controller is disabled.
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
            wav::roundtrip(&rvc_wav, |samples, sr| {
                let opts = if controller_enabled {
                    let analysis = foni_analyse::analyse_fast(samples, sr);
                    // Try policy script first, fall back to compiled controller
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
        rvc_wav
    };

    // Store in cache.
    {
        let mut cache = state.0.wav_cache.lock().await;
        cache.put(key, final_wav.clone());
        tracing::debug!("cached synthesis ({} entries)", cache.len());
    }

    let ms = t_start.elapsed().as_millis() as u64;
    state.0.sessions.release_active();
    state.0.sessions.record_latency(ms).await;
    tracing::debug!(synthesis_ms = ms);

    wav_response(final_wav)
}

fn wav_response(bytes: Vec<u8>) -> Result<Response, (StatusCode, String)> {
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .body(Body::from(bytes))
        .expect("infallible"))
}
