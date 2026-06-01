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
use std::path::Path;

use crate::{state::AppState, wav::encode_wav};
use foni_analyse::decode_wav;

// HuBERT stride at 16 kHz. Repeat-by-2 brings 50 Hz → 100 Hz to match the generator.
pub(crate) const CONTENTVEC_HOP: usize = 320;
pub(crate) const GENERATOR_SR: u32 = 40_000;
// Mel-scale F0 bounds from the bandit training config.
pub(crate) const F0_MIN: f32 = 50.0;
pub(crate) const F0_MAX: f32 = 1100.0;
pub(crate) const NOISE_CHANNELS: usize = 192;

#[derive(Deserialize)]
pub struct ConvertRequest {
    pub audio_data: String,
    pub model: Option<String>,
    #[serde(default)]
    pub speaker_id: i64,
    #[serde(default)]
    pub f0_up_key: i32,
}

pub(crate) fn load_session(path: &Path) -> Result<ort::session::Session, String> {
    ort::session::Session::builder()
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| e.to_string())
}

/// Extract ContentVec features. Returns raw [t_prime, 768] (before repeat-by-2).
pub(crate) fn extract_contentvec(
    audio_16k: &[f32],
    session: &mut ort::session::Session,
) -> Result<(Vec<f32>, usize), String> {
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
    Ok((data.to_vec(), t_prime))
}

/// Repeat each frame twice (50 Hz → 100 Hz) to match the generator hop.
pub(crate) fn repeat_by_2(feats: &[f32], t_prime: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; t_prime * 2 * 768];
    for ti in 0..t_prime {
        for di in 0..768 {
            let v = feats[ti * 768 + di];
            out[(ti * 2) * 768 + di] = v;
            out[(ti * 2 + 1) * 768 + di] = v;
        }
    }
    out
}

/// Backward-compat wrapper: extract + optional voice-index blend + repeat.
pub(crate) fn run_contentvec(
    audio_16k: &[f32],
    session: &mut ort::session::Session,
    voice_idx: Option<&crate::voice_index::VoiceIndex>,
    index_rate: f32,
) -> Result<Vec<f32>, String> {
    let (raw, t_prime) = extract_contentvec(audio_16k, session)?;
    let blended = if let Some(idx) = voice_idx {
        idx.blend(&raw, t_prime, index_rate)
    } else {
        raw
    };
    Ok(repeat_by_2(&blended, t_prime))
}

// Fixed mel-spectrogram params required by the RMVPE model training config.
const RMVPE_N_FFT: usize = 1024;
const RMVPE_HOP: usize = 160;
const RMVPE_N_MELS: usize = 128;
const RMVPE_FMIN: f32 = 30.0;
const RMVPE_FMAX: f32 = 8_000.0;
// 360-bin pitch tokenisation: cents = 20*i + 1997.379...
const RMVPE_BINS: usize = 360;
const RMVPE_CENTS_OFFSET: f32 = 1_997.379_4;
const RMVPE_CENTS_STEP: f32 = 20.0;

/// Center-padded mel spectrogram → `[1, 128, T_frames]` flat row-major buffer.
pub(crate) fn audio_to_mel(audio: &[f32]) -> (Vec<f32>, usize) {
    use rustfft::{num_complex::Complex, FftPlanner};
    use std::f32::consts::PI;

    let n_fft = RMVPE_N_FFT;
    let hop = RMVPE_HOP;
    let sr = 16_000u32;
    let n_spec = n_fft / 2 + 1;

    let window: Vec<f32> = (0..n_fft)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (n_fft - 1) as f32).cos()))
        .collect();

    let mel_hz = |hz: f32| 2595.0 * (1.0 + hz / 700.0).log10();
    let imel = |m: f32| 700.0 * (10f32.powf(m / 2595.0) - 1.0);
    let mel_lo = mel_hz(RMVPE_FMIN);
    let mel_hi = mel_hz(RMVPE_FMAX);
    let mel_pts: Vec<f32> = (0..=RMVPE_N_MELS + 1)
        .map(|i| imel(mel_lo + i as f32 * (mel_hi - mel_lo) / (RMVPE_N_MELS + 1) as f32))
        .collect();
    let fft_freqs: Vec<f32> = (0..n_spec)
        .map(|k| k as f32 * sr as f32 / n_fft as f32)
        .collect();
    let mut fb = vec![vec![0.0f32; n_spec]; RMVPE_N_MELS];
    for m in 0..RMVPE_N_MELS {
        let (f0, fc, f1) = (mel_pts[m], mel_pts[m + 1], mel_pts[m + 2]);
        for (k, &f) in fft_freqs.iter().enumerate() {
            fb[m][k] = if f >= f0 && f <= fc {
                (f - f0) / (fc - f0)
            } else if f > fc && f <= f1 {
                (f1 - f) / (f1 - fc)
            } else {
                0.0
            };
        }
    }

    let pad = n_fft / 2;
    let mut padded = vec![0.0f32; pad + audio.len() + pad];
    padded[pad..pad + audio.len()].copy_from_slice(audio);

    let n_frames = audio.len().div_ceil(hop) + 1;
    let mut planner: FftPlanner<f32> = FftPlanner::new();
    let fft = planner.plan_fft_forward(n_fft);

    // Row-major: [n_mels, n_frames] then reshaped to [1, n_mels, n_frames]
    let mut out = vec![0.0f32; RMVPE_N_MELS * n_frames];
    let mut buf = vec![Complex::default(); n_fft];
    for fi in 0..n_frames {
        let start = fi * hop;
        buf.iter_mut().enumerate().for_each(|(i, c)| {
            let s = padded.get(start + i).copied().unwrap_or(0.0);
            *c = Complex {
                re: s * window[i],
                im: 0.0,
            };
        });
        fft.process(&mut buf);
        let mags: Vec<f32> = buf[..n_spec].iter().map(|c| c.norm()).collect();
        for m in 0..RMVPE_N_MELS {
            let mel: f32 = fb[m].iter().zip(&mags).map(|(&w, &x)| w * x).sum();
            // RMVPE was trained on log-mel (log(clamp(mel, 1e-5))). Linear mel
            // produces wrong-scale salience peaks → erratic F0 → parrot voice.
            out[m * n_frames + fi] = mel.max(1e-5_f32).ln();
        }
    }

    // RMVPE UNet has 5 encoder stages each with stride 2 along time → T must be
    // divisible by 32 for the skip connections to match.
    let padded_frames = n_frames.div_ceil(32) * 32;
    if padded_frames > n_frames {
        out.resize(RMVPE_N_MELS * padded_frames, 0.0);
        // Zero-filled columns are already correct; re-layout: currently [n_mels, n_frames],
        // need [n_mels, padded_frames] — existing data is contiguous per mel row so we must
        // shift rows apart. Easiest: build new buffer.
        let mut padded_out = vec![0.0f32; RMVPE_N_MELS * padded_frames];
        for m in 0..RMVPE_N_MELS {
            padded_out[m * padded_frames..m * padded_frames + n_frames]
                .copy_from_slice(&out[m * n_frames..(m + 1) * n_frames]);
        }
        return (padded_out, n_frames); // return original n_frames for downstream use
    }

    (out, n_frames)
}

/// RMVPE local-average-cents decoder: salience `[T, 360]` → F0 Hz per frame.
pub(crate) fn salience_to_hz(salience: &[f32], n_frames: usize, threshold: f32) -> Vec<f32> {
    // cents_mapping with 4-element padding on each side (as in Python)
    let cents: Vec<f32> = (0..RMVPE_BINS + 8)
        .map(|i| {
            if !(4..RMVPE_BINS + 4).contains(&i) {
                0.0
            } else {
                RMVPE_CENTS_OFFSET + RMVPE_CENTS_STEP * (i as f32 - 4.0)
            }
        })
        .collect();

    (0..n_frames)
        .map(|t| {
            let row = &salience[t * RMVPE_BINS..(t + 1) * RMVPE_BINS];
            let (center, &max_val) = row
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                .unwrap_or((0, &0.0));
            if max_val <= threshold {
                return 0.0;
            }
            let lo = center + 4; // cents array is padded by 4
            let (product, weight): (f32, f32) = (0..9)
                .map(|d| {
                    let bi = (center + d).saturating_sub(4).min(RMVPE_BINS - 1);
                    (row[bi] * cents[lo + d - 4], row[bi])
                })
                .fold((0.0, 0.0), |(p, w), (pi, wi)| (p + pi, w + wi));
            if weight == 0.0 {
                0.0
            } else {
                10.0 * 2.0f32.powf(product / weight / 1200.0)
            }
        })
        .collect()
}

pub(crate) fn run_rmvpe(
    audio_16k: &[f32],
    t_phone: usize,
    f0_up_key: i32,
    session: &mut ort::session::Session,
) -> Result<(Vec<i64>, Vec<f32>), String> {
    use ort::value::Tensor;

    let (mel_flat, n_frames) = audio_to_mel(audio_16k);
    let padded_frames = mel_flat.len() / RMVPE_N_MELS; // may be > n_frames if padded to multiple of 32

    let input = Tensor::<f32>::from_array(([1usize, RMVPE_N_MELS, padded_frames], mel_flat))
        .map_err(|e| e.to_string())?;

    let outputs = session
        .run(ort::inputs!["input" => input])
        .map_err(|e| e.to_string())?;

    let (shape, data) = outputs["output"]
        .try_extract_tensor::<f32>()
        .map_err(|e| e.to_string())?;

    // Output may be [B, T, 360] or [B, 360, T] — detect by checking dim 1 vs 2
    let salience: Vec<f32> = if shape[2] as usize == RMVPE_BINS {
        data.to_vec() // already [B, T, 360] → take [T, 360] slice
    } else {
        // [B, 360, T] → transpose to [T, 360]
        let t = shape[2] as usize;
        let mut out = vec![0.0f32; n_frames * RMVPE_BINS];
        for ti in 0..t.min(n_frames) {
            for b in 0..RMVPE_BINS {
                out[ti * RMVPE_BINS + b] = data[b * t + ti];
            }
        }
        out
    };
    let model_frames = shape[1] as usize;

    let f0_raw = salience_to_hz(&salience, model_frames.min(n_frames), 0.03);
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

// The generator was exported with T=200 as the trace size. The TorchScript exporter
// bakes the attention K-length from the trace, so only T=200 works without shape errors.
// Process in 200-frame chunks and concatenate audio output.
pub(crate) const GENERATOR_CHUNK: usize = 200;

pub(crate) fn run_generator_chunk(
    phone: &[f32],
    pitch: &[i64],
    pitchf: &[f32],
    speaker_id: i64,
    session: &mut ort::session::Session,
) -> Result<Vec<f32>, String> {
    use ort::value::Tensor;
    use rand::Rng;

    let t = GENERATOR_CHUNK;
    let noise: Vec<f32> = (0..NOISE_CHANNELS * t)
        .map(|_| rand::thread_rng().gen::<f32>() * 0.1)
        .collect();

    let phone_t =
        Tensor::<f32>::from_array(([1usize, t, 768], phone.to_vec())).map_err(|e| e.to_string())?;
    let lengths =
        Tensor::<i64>::from_array(([1usize], vec![t as i64])).map_err(|e| e.to_string())?;
    let pitch_t =
        Tensor::<i64>::from_array(([1usize, t], pitch.to_vec())).map_err(|e| e.to_string())?;
    let pitchf_t =
        Tensor::<f32>::from_array(([1usize, t], pitchf.to_vec())).map_err(|e| e.to_string())?;
    let ds = Tensor::<i64>::from_array(([1usize], vec![speaker_id])).map_err(|e| e.to_string())?;
    let rnd = Tensor::<f32>::from_array(([1usize, NOISE_CHANNELS, t], noise))
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

pub(crate) fn run_generator(
    phone: Vec<f32>,
    t_phone: usize,
    pitch: Vec<i64>,
    pitchf: Vec<f32>,
    speaker_id: i64,
    session: &mut ort::session::Session,
) -> Result<Vec<f32>, String> {
    // Pad inputs to a multiple of GENERATOR_CHUNK, then process chunk-by-chunk.
    let n_chunks = t_phone.div_ceil(GENERATOR_CHUNK);
    let t_padded = n_chunks * GENERATOR_CHUNK;

    let mut phone_p = phone;
    phone_p.resize(t_padded * 768, 0.0);
    let mut pitch_p = pitch;
    pitch_p.resize(t_padded, 1);
    let mut pitchf_p = pitchf;
    pitchf_p.resize(t_padded, 0.0);

    // Probe first chunk to learn samples_per_chunk (export-time constant: 80000/200=400).
    let samples_per_chunk = {
        let out = run_generator_chunk(
            &phone_p[..GENERATOR_CHUNK * 768],
            &pitch_p[..GENERATOR_CHUNK],
            &pitchf_p[..GENERATOR_CHUNK],
            speaker_id,
            session,
        )?;
        out.len() / GENERATOR_CHUNK
    };

    let mut audio_out = Vec::with_capacity(t_phone * samples_per_chunk);

    // First chunk already processed above — re-process from start for simplicity
    for chunk in 0..n_chunks {
        let s = chunk * GENERATOR_CHUNK;
        let chunk_audio = run_generator_chunk(
            &phone_p[s * 768..(s + GENERATOR_CHUNK) * 768],
            &pitch_p[s..s + GENERATOR_CHUNK],
            &pitchf_p[s..s + GENERATOR_CHUNK],
            speaker_id,
            session,
        )?;
        // Trim the last chunk — it may contain silence from zero-padding.
        let voiced_frames = if chunk == n_chunks - 1 {
            t_phone - chunk * GENERATOR_CHUNK
        } else {
            GENERATOR_CHUNK
        };
        let keep = voiced_frames * samples_per_chunk;
        audio_out.extend_from_slice(&chunk_audio[..keep.min(chunk_audio.len())]);
    }

    Ok(audio_out)
}

/// 4th-order Butterworth highpass at 48 Hz (2 × 2nd-order biquads cascaded).
/// Matches rvc_python pipeline.py: `signal.butter(N=5, Wn=48, btype="high", fs=16000)`.
/// Removes DC and sub-bass before ContentVec and RMVPE.
pub(crate) fn highpass_48hz(audio: &mut [f32]) {
    use crate::dsp::filters::Biquad;
    let mut s1 = Biquad::highpass(48.0, 16_000);
    let mut s2 = Biquad::highpass(48.0, 16_000);
    for x in audio.iter_mut() {
        *x = s2.process_sample(s1.process_sample(*x));
    }
}

/// Frame-level RMS envelope matching: scale `output` so its loudness tracks `input`.
/// Matches rvc_python change_rms(data1, sr1, data2, sr2, rate=rms_mix_rate).
/// rate=0.45 → output *= rms_in^0.55 * rms_out^(-0.55) per frame.
pub(crate) fn apply_rms_mix(input_16k: &[f32], output: &mut [f32], output_sr: u32, rate: f32) {
    let frame_in = 16_000_usize / 2; // 0.5 s frames at 16 kHz
    let frame_out = output_sr as usize / 2; // 0.5 s frames at output SR

    let rms = |s: &[f32], hop: usize| -> Vec<f32> {
        let n = s.len().div_ceil(hop);
        (0..n)
            .map(|i| {
                let sl = &s[i * hop..((i + 1) * hop).min(s.len())];
                (sl.iter().map(|x| x * x).sum::<f32>() / sl.len() as f32).sqrt()
            })
            .collect()
    };

    let rms_in = rms(input_16k, frame_in);
    let rms_out = rms(output, frame_out);

    let interp = |v: &[f32], target_len: usize| -> Vec<f32> {
        if v.is_empty() {
            return vec![1.0; target_len];
        }
        (0..target_len)
            .map(|i| {
                let src = i as f32 * (v.len() as f32 - 1.0) / (target_len as f32 - 1.0).max(1.0);
                let lo = src.floor() as usize;
                let hi = (lo + 1).min(v.len() - 1);
                v[lo] + (src - lo as f32) * (v[hi] - v[lo])
            })
            .collect()
    };

    let n = output.len();
    let ri = interp(&rms_in, n);
    let ro = interp(&rms_out, n);

    for (i, x) in output.iter_mut().enumerate() {
        let env_in = ri[i].max(1e-6);
        let env_out = ro[i].max(1e-6);
        *x *= env_in.powf(1.0 - rate) * env_out.powf(rate - 1.0);
    }
}

pub(crate) fn resample_to_16k(samples: &[f32], src_sr: u32) -> Vec<f32> {
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
            .unwrap_or_else(|| guard.as_deref().unwrap_or("sidorovich").to_string())
    };

    // Ensure sessions are loaded for this model (no-op if already in pool).
    crate::sessions::ensure(&state, &model_name)
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;

    let bytes = B64
        .decode(&req.audio_data)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("base64: {e}")))?;
    let wav =
        decode_wav(&bytes).map_err(|e| (StatusCode::BAD_REQUEST, format!("WAV decode: {e}")))?;
    let mut audio_16k = resample_to_16k(&wav.samples, wav.sample_rate);
    highpass_48hz(&mut audio_16k);
    let t_prime = audio_16k.len() / CONTENTVEC_HOP;
    let t_phone = t_prime * 2;

    if t_phone == 0 {
        return Err((StatusCode::BAD_REQUEST, "Audio too short".to_string()));
    }

    // Hold the session pool lock for the duration of inference.
    let mut pool_guard = state.0.sessions.acquire().await;
    let pool = pool_guard.session.as_mut().expect("sessions loaded above");

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

    let vi = state.0.voice_index.read().await;
    let params = state.0.params.read().await;
    let phone = stage!(
        "ContentVec",
        run_contentvec(
            &audio_16k,
            &mut pool.contentvec,
            vi.as_ref(),
            params.index_rate
        )
    );
    let (pitch, pitchf) = stage!(
        "RMVPE",
        run_rmvpe(&audio_16k, t_phone, req.f0_up_key, &mut pool.rmvpe)
    );
    let mut audio_out = stage!(
        "Generator",
        run_generator(
            phone,
            t_phone,
            pitch,
            pitchf,
            req.speaker_id,
            &mut pool.generator
        )
    );
    apply_rms_mix(&audio_16k, &mut audio_out, GENERATOR_SR, 0.45);

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
