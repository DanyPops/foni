/// POST /synthesize — text → SSML → espeak → DSP → WAV, with LRU cache.
///
/// The RVC voice conversion pipeline has been removed. Voice identity
/// will be provided by Fish Speech fine-tuned model (FON-TSK-167).
/// Currently: espeak raw synthesis + reactive DSP correction.
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

use super::process::WireOpts;

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
    for v in [
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
    ] {
        if let Some(val) = v {
            h.update(val.to_le_bytes());
        }
    }
    h.finalize().into()
}

/// Prepare text for espeak: apply SSML annotation if prosody is enabled.
fn prepare_text(req: &SynthRequest) -> (String, bool) {
    if !req.prosody {
        return (req.text.clone(), false);
    }
    let annotated = ssml::annotate_with_prosody(&req.text);
    (annotated, true)
}

fn espeak(text: &str, voice: &str, speed: u32, markup: bool) -> Result<Vec<u8>, String> {
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
    if markup {
        cmd.arg("-m");
    }
    cmd.arg(text);
    let status = cmd.status().map_err(|e| format!("espeak-ng: {e}"))?;
    if !status.success() {
        return Err("espeak-ng exited non-zero".into());
    }
    std::fs::read(tmp.path()).map_err(|e| format!("read espeak output: {e}"))
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

    // Prepare text
    let (synth_text, markup) = prepare_text(&req);

    // Espeak synthesis
    let voice = req.voice.clone();
    let speed = req.speed;
    let espeak_bytes =
        tokio::task::spawn_blocking(move || espeak(&synth_text, &voice, speed, markup))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
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
            wav::roundtrip(&espeak_bytes, |samples, sr| {
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
        espeak_bytes
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
