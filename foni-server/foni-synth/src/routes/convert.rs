/// POST /convert — ContentVec → RMVPE → Generator. Returns raw WAV bytes (audio/wav).
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::Response,
    Json,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::{state::AppState, wav::encode_wav};
use foni_analyse::decode_wav;

// HuBERT stride at 16 kHz. Repeat-by-2 brings 50 Hz → 100 Hz to match the generator.
const CONTENTVEC_HOP: usize = 320;
const GENERATOR_SR: u32 = 40_000;
// Mel-scale F0 bounds from the bandit training config.
const F0_MIN: f32 = 50.0;
const F0_MAX: f32 = 1100.0;
const NOISE_CHANNELS: usize = 192;

#[derive(Deserialize)]
pub struct ConvertRequest {
    pub audio_data: String,
    pub model: Option<String>,
    #[serde(default)]
    pub speaker_id: i64,
    #[serde(default)]
    pub f0_up_key: i32,
}

fn find_pretrained(filename: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("FONI_MODELS_DIR") {
        let p = PathBuf::from(dir).join("pretrained").join(filename);
        if p.exists() {
            return Some(p);
        }
    }
    for base in &["../../rvc/models/pretrained", "../rvc/models/pretrained"] {
        let p = PathBuf::from(base).join(filename);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn generator_path(state: &AppState, model_name: &str) -> PathBuf {
    state
        .0
        .models_dir
        .join(model_name)
        .join("onnx")
        .join("generator.onnx")
}

fn load_session(path: &Path) -> Result<ort::session::Session, String> {
    ort::session::Session::builder()
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| e.to_string())
}

fn run_contentvec(
    audio_16k: &[f32],
    session: &mut ort::session::Session,
) -> Result<Vec<f32>, String> {
    use ort::value::Tensor;

    let t = audio_16k.len();
    let input = Tensor::<f32>::from_array(([1usize, 1, t], audio_16k.to_vec()))
        .map_err(|e| e.to_string())?;

    let outputs = session
        .run(ort::inputs!["source" => input])
        .map_err(|e| e.to_string())?;

    let (shape, data) = outputs["embed"]
        .try_extract_tensor::<f32>()
        .map_err(|e| e.to_string())?;

    let t_prime = shape[1] as usize;
    assert_eq!(shape[2], 768, "ContentVec must output 768-dim features");

    let mut repeated = vec![0.0f32; t_prime * 2 * 768];
    for ti in 0..t_prime {
        for di in 0..768 {
            let v = data[ti * 768 + di];
            repeated[(ti * 2) * 768 + di] = v;
            repeated[(ti * 2 + 1) * 768 + di] = v;
        }
    }
    Ok(repeated)
}

fn run_rmvpe(
    audio_16k: &[f32],
    t_phone: usize,
    f0_up_key: i32,
    session: &mut ort::session::Session,
) -> Result<(Vec<i64>, Vec<f32>), String> {
    use ort::value::Tensor;

    let t = audio_16k.len();
    let input =
        Tensor::<f32>::from_array(([1usize, t], audio_16k.to_vec())).map_err(|e| e.to_string())?;

    let outputs = session
        .run(ort::inputs!["input" => input])
        .map_err(|e| e.to_string())?;

    let (_, data) = outputs["output"]
        .try_extract_tensor::<f32>()
        .map_err(|e| e.to_string())?;

    let f0_raw: Vec<f32> = data.to_vec();
    let semitone_scale = 2.0f32.powf(f0_up_key as f32 / 12.0);

    let pitchf: Vec<f32> = (0..t_phone)
        .map(|i| {
            let src = (i as f32 * f0_raw.len() as f32 / t_phone as f32) as usize;
            f0_raw[src.min(f0_raw.len() - 1)] * semitone_scale
        })
        .collect();

    let f0_mel_min = 1127.0 * (1.0 + F0_MIN / 700.0).ln();
    let f0_mel_max = 1127.0 * (1.0 + F0_MAX / 700.0).ln();
    let pitch: Vec<i64> = pitchf
        .iter()
        .map(|&hz| {
            if hz <= 0.0 {
                return 1i64;
            }
            let mel = 1127.0 * (1.0 + hz / 700.0).ln();
            ((mel - f0_mel_min) * 254.0 / (f0_mel_max - f0_mel_min) + 1.0)
                .round()
                .clamp(1.0, 255.0) as i64
        })
        .collect();

    Ok((pitch, pitchf))
}

fn run_generator(
    phone: Vec<f32>,
    t_phone: usize,
    pitch: Vec<i64>,
    pitchf: Vec<f32>,
    speaker_id: i64,
    session: &mut ort::session::Session,
) -> Result<Vec<f32>, String> {
    use ort::value::Tensor;
    use rand::Rng;

    let noise: Vec<f32> = (0..NOISE_CHANNELS * t_phone)
        .map(|_| rand::thread_rng().gen::<f32>() * 0.1)
        .collect();

    let phone_t =
        Tensor::<f32>::from_array(([1usize, t_phone, 768], phone)).map_err(|e| e.to_string())?;
    let lengths =
        Tensor::<i64>::from_array(([1usize], vec![t_phone as i64])).map_err(|e| e.to_string())?;
    let pitch_t =
        Tensor::<i64>::from_array(([1usize, t_phone], pitch)).map_err(|e| e.to_string())?;
    let pitchf_t =
        Tensor::<f32>::from_array(([1usize, t_phone], pitchf)).map_err(|e| e.to_string())?;
    let ds = Tensor::<i64>::from_array(([1usize], vec![speaker_id])).map_err(|e| e.to_string())?;
    let rnd = Tensor::<f32>::from_array(([1usize, NOISE_CHANNELS, t_phone], noise))
        .map_err(|e| e.to_string())?;

    let outputs = session
        .run(ort::inputs![
            "phone"         => phone_t,
            "phone_lengths" => lengths,
            "pitch"         => pitch_t,
            "pitchf"        => pitchf_t,
            "ds"            => ds,
            "rnd"           => rnd,
        ])
        .map_err(|e| e.to_string())?;

    let (_, data) = outputs["audio"]
        .try_extract_tensor::<f32>()
        .map_err(|e| e.to_string())?;

    Ok(data.to_vec())
}

fn resample_to_16k(samples: &[f32], src_sr: u32) -> Vec<f32> {
    if src_sr == 16_000 {
        return samples.to_vec();
    }
    let ratio = src_sr as f64 / 16_000.0;
    let out_len = (samples.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let lo = src.floor() as usize;
            let hi = (lo + 1).min(samples.len() - 1);
            let frac = src - lo as f64;
            samples[lo] * (1.0 - frac as f32) + samples[hi] * frac as f32
        })
        .collect()
}

pub async fn convert(
    State(state): State<AppState>,
    Json(req): Json<ConvertRequest>,
) -> Result<Response, (StatusCode, String)> {
    let model_name = {
        let guard = state.0.current_model.read().await;
        req.model
            .clone()
            .unwrap_or_else(|| guard.as_deref().unwrap_or("bandit").to_string())
    };

    let cv_path = find_pretrained("contentvec-768-l12.onnx").ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "ContentVec ONNX not found. Run: python3 rvc/export_contentvec_onnx.py".to_string(),
    ))?;
    let rmvpe_path = find_pretrained("rmvpe.onnx").ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "RMVPE ONNX not found. Download from HuggingFace lj1995/VoiceConversionWebUI".to_string(),
    ))?;
    let gen_path = generator_path(&state, &model_name);
    if !gen_path.exists() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "Generator ONNX not found at {}. Run: python3 rvc/export_onnx.py {}",
                gen_path.display(),
                model_name
            ),
        ));
    }

    let bytes = B64
        .decode(&req.audio_data)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("base64: {e}")))?;
    let wav =
        decode_wav(&bytes).map_err(|e| (StatusCode::BAD_REQUEST, format!("WAV decode: {e}")))?;
    let audio_16k = resample_to_16k(&wav.samples, wav.sample_rate);
    let t_prime = audio_16k.len() / CONTENTVEC_HOP;
    let t_phone = t_prime * 2;

    if t_phone == 0 {
        return Err((StatusCode::BAD_REQUEST, "Audio too short".to_string()));
    }

    macro_rules! stage {
        ($label:expr, $expr:expr) => {
            $expr.map_err(|e: String| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("{}: {e}", $label),
                )
            })?
        };
    }

    let mut cv_sess = stage!("ContentVec load", load_session(&cv_path));
    let mut rmvpe_sess = stage!("RMVPE load", load_session(&rmvpe_path));
    let mut gen_sess = stage!("Generator load", load_session(&gen_path));

    let phone = stage!("ContentVec", run_contentvec(&audio_16k, &mut cv_sess));
    let (pitch, pitchf) = stage!(
        "RMVPE",
        run_rmvpe(&audio_16k, t_phone, req.f0_up_key, &mut rmvpe_sess)
    );
    let audio_out = stage!(
        "Generator",
        run_generator(phone, t_phone, pitch, pitchf, req.speaker_id, &mut gen_sess)
    );

    let wav_bytes = encode_wav(&audio_out, GENERATOR_SR).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("WAV encode: {e}"),
        )
    })?;

    // TS reads `await resp.arrayBuffer()` — must be raw bytes, not JSON.
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .body(Body::from(wav_bytes))
        .expect("infallible"))
}

/// Run a dummy forward pass to verify the generator ONNX loads and produces audio.
pub fn validate_generator_onnx(path: &Path) -> Result<Vec<usize>, String> {
    use ort::value::Tensor;

    let mut session = load_session(path)?;

    // T=200 matches the export trace shape — dynamic axes allow other lengths at runtime.
    let t: usize = 200;
    let phone = Tensor::<f32>::from_array(([1usize, t, 768], vec![0.0f32; t * 768]))
        .map_err(|e| e.to_string())?;
    let lengths =
        Tensor::<i64>::from_array(([1usize], vec![t as i64])).map_err(|e| e.to_string())?;
    let pitch =
        Tensor::<i64>::from_array(([1usize, t], vec![100i64; t])).map_err(|e| e.to_string())?;
    let pitchf =
        Tensor::<f32>::from_array(([1usize, t], vec![220.0f32; t])).map_err(|e| e.to_string())?;
    let ds = Tensor::<i64>::from_array(([1usize], vec![0i64])).map_err(|e| e.to_string())?;
    let rnd = Tensor::<f32>::from_array((
        [1usize, NOISE_CHANNELS, t],
        vec![0.0f32; NOISE_CHANNELS * t],
    ))
    .map_err(|e| e.to_string())?;

    let outputs = session
        .run(ort::inputs![
            "phone"         => phone,
            "phone_lengths" => lengths,
            "pitch"         => pitch,
            "pitchf"        => pitchf,
            "ds"            => ds,
            "rnd"           => rnd,
        ])
        .map_err(|e| e.to_string())?;

    let (shape, _) = outputs["audio"]
        .try_extract_tensor::<f32>()
        .map_err(|e| e.to_string())?;

    Ok(shape.iter().map(|&d| d as usize).collect())
}
