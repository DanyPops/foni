/// http_integration — foni-synth HTTP routes against a real bound server.
///
/// Starts an axum server on a random port (no process required), hits all
/// three critical endpoints, and asserts on the responses.
///
/// This is the missing Rust↔TS seam test: proves the HTTP layer works
/// end-to-end without relying on a separately-launched process.
///
/// cargo test -p foni-synth --test http_integration -- --nocapture
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use tokio::net::TcpListener;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Minimal 16-bit mono WAV with silence.
fn silence_wav(sample_rate: u32, duration_ms: u32) -> Vec<u8> {
    let n = (sample_rate as f32 * duration_ms as f32 / 1000.0) as usize;
    foni_synth::wav::encode_wav(&vec![0.0f32; n], sample_rate).unwrap()
}

/// WAV containing a 440 Hz sine wave.
fn sine_wav(sample_rate: u32, duration_ms: u32) -> Vec<u8> {
    use std::f32::consts::PI;
    let n = (sample_rate as f32 * duration_ms as f32 / 1000.0) as usize;
    let samples: Vec<f32> = (0..n)
        .map(|i| (2.0 * PI * 440.0 * i as f32 / sample_rate as f32).sin() * 0.5)
        .collect();
    foni_synth::wav::encode_wav(&samples, sample_rate).unwrap()
}

/// RMS of decoded WAV bytes.
fn wav_rms(bytes: &[u8]) -> f32 {
    let wav = foni_synth::wav::decode_wav(bytes).unwrap();
    let s = &wav.samples;
    (s.iter().map(|&x| x * x).sum::<f32>() / s.len() as f32).sqrt()
}

/// Start the server on a random port and return the base URL.
async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = foni_synth::build_router().await;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn process_changes_audio() {
    let base = start_server().await;
    let input = sine_wav(22050, 500);
    let rms_in = wav_rms(&input);

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/process"))
        .json(&json!({ "audio_data": B64.encode(&input) }))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_success(),
        "POST /process failed: {}",
        resp.status()
    );

    let body: Value = resp.json().await.unwrap();
    let audio_b64 = body["audio_data"].as_str().expect("missing audio_data");
    let output = B64.decode(audio_b64).unwrap();

    let rms_out = wav_rms(&output);
    println!("RMS: in={rms_in:.4} out={rms_out:.4}");

    // DSP chain (loudnorm, compression) must produce different audio
    assert_ne!(input, output, "POST /process must transform the audio");
    assert!(output.len() > 44, "output WAV must be non-trivial");
}

#[tokio::test]
async fn breath_returns_non_silent_audio() {
    let base = start_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/breath"))
        .json(&json!({ "duration_ms": 120, "sample_rate": 22050 }))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_success(),
        "POST /breath failed: {}",
        resp.status()
    );

    let body: Value = resp.json().await.unwrap();
    let audio_b64 = body["audio_data"].as_str().expect("missing audio_data");
    let output = B64.decode(audio_b64).unwrap();
    let rms = wav_rms(&output);

    println!("Breath RMS: {rms:.6}");
    assert!(rms > 1e-4, "breath WAV must be non-silent, got RMS={rms}");
}

#[tokio::test]
async fn analyse_returns_valid_metrics() {
    let base = start_server().await;
    let input = sine_wav(22050, 500);

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/analyse"))
        .json(&json!({ "audio_data": B64.encode(&input) }))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_success(),
        "POST /analyse failed: {}",
        resp.status()
    );

    let body: Value = resp.json().await.unwrap();
    let analysis = &body["analysis"];

    let rms_db = analysis["loudness"]["rms_db"]
        .as_f64()
        .expect("missing rms_db");
    let dur = analysis["temporal"]["duration_secs"]
        .as_f64()
        .expect("missing duration_secs");

    println!("Analysis: rms={rms_db:.1}dBFS  dur={dur:.3}s");

    assert!(rms_db > -60.0, "RMS should not be silence: {rms_db}");
    assert!(rms_db < 0.0, "RMS must be negative dBFS: {rms_db}");
    assert!((dur - 0.5).abs() < 0.05, "Duration should be ~0.5s: {dur}");
}

#[tokio::test]
async fn models_returns_json_list() {
    let base = start_server().await;
    let resp = reqwest::get(format!("{base}/models")).await.unwrap();
    assert!(resp.status().is_success());
    let body: Value = resp.json().await.unwrap();
    assert!(body["models"].is_array(), "models must be an array");
}

#[tokio::test]
async fn params_roundtrip() {
    let base = start_server().await;
    let client = reqwest::Client::new();

    // Read defaults
    let resp = client.get(format!("{base}/params")).send().await.unwrap();
    assert!(resp.status().is_success());
    let defaults: Value = resp.json().await.unwrap();
    let orig_key: i64 = defaults["f0up_key"].as_i64().unwrap_or(-2);

    // Patch
    let new_key = orig_key + 1;
    let resp = client
        .post(format!("{base}/params"))
        .json(&json!({ "f0up_key": new_key }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let patched: Value = resp.json().await.unwrap();
    assert_eq!(patched["f0up_key"].as_i64(), Some(new_key));
}
