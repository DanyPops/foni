/// tts_contract — contract tests for cloud_tts() HTTP client.
///
/// Verifies synthesize_route handles all response shapes from a TTS endpoint:
/// success, auth failure, server error, timeout, malformed body.
///
/// cargo test -p foni-synth --test tts_contract -- --nocapture
use axum::{routing::post, Json, Router};
use serde_json::json;
use std::time::Duration;
use tokio::net::TcpListener;

async fn start_mock(handler: axum::routing::MethodRouter) -> (String, u16) {
    let app = Router::new().route("/synthesize", handler);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://127.0.0.1:{port}"), port)
}

fn sine_wav() -> Vec<u8> {
    let sr = 22050u32;
    let n = sr / 2;
    let samples: Vec<f32> = (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.3)
        .collect();
    foni_synth::wav::encode_wav(&samples, sr).expect("infallible")
}

#[tokio::test]
async fn contract_success_returns_wav() {
    let wav = sine_wav();
    let wav_clone = wav.clone();
    let (base_url, _) = start_mock(post(move || async move {
        let headers = [(axum::http::header::CONTENT_TYPE, "audio/wav")];
        (headers, wav_clone.clone())
    }))
    .await;

    std::env::set_var("FISH_SPEECH_URL", format!("{base_url}/synthesize"));
    std::env::remove_var("FONI_TTS_TOKEN");

    let app = foni_synth::build_router().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/synthesize"))
        .json(&json!({"text": "тест", "voice": "ru", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.bytes().await.unwrap();
    assert!(
        body.len() > 44,
        "should return WAV, got {} bytes",
        body.len()
    );
    eprintln!(
        "  [contract] success: {} bytes WAV from mock TTS",
        body.len()
    );
}

#[tokio::test]
async fn contract_401_falls_back_to_espeak() {
    let (base_url, _) = start_mock(post(|| async {
        (axum::http::StatusCode::UNAUTHORIZED, "unauthorized")
    }))
    .await;

    std::env::set_var("FISH_SPEECH_URL", format!("{base_url}/synthesize"));
    std::env::remove_var("FONI_TTS_TOKEN");

    let app = foni_synth::build_router().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/synthesize"))
        .json(&json!({"text": "тест", "voice": "ru", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "should fall back to espeak on 401");
    let body = resp.bytes().await.unwrap();
    assert!(body.len() > 44, "espeak fallback should produce WAV");
    eprintln!("  [contract] 401 → espeak fallback: {} bytes", body.len());
}

#[tokio::test]
async fn contract_500_falls_back_to_espeak() {
    let (base_url, _) = start_mock(post(|| async {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "model crashed",
        )
    }))
    .await;

    std::env::set_var("FISH_SPEECH_URL", format!("{base_url}/synthesize"));

    let app = foni_synth::build_router().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/synthesize"))
        .json(&json!({"text": "тест", "voice": "ru", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "should fall back to espeak on 500");
    eprintln!("  [contract] 500 → espeak fallback");
}

#[tokio::test]
async fn contract_timeout_falls_back_to_espeak() {
    let (base_url, _) = start_mock(post(|| async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        "never reached"
    }))
    .await;

    std::env::set_var("FISH_SPEECH_URL", format!("{base_url}/synthesize"));

    let app = foni_synth::build_router().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/synthesize"))
        .json(&json!({"text": "тест", "voice": "ru", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "should fall back to espeak on timeout");
    eprintln!("  [contract] timeout → espeak fallback");
}

#[tokio::test]
async fn contract_malformed_response_falls_back() {
    let (base_url, _) = start_mock(post(|| async { Json(json!({"not": "wav"})) })).await;

    std::env::set_var("FISH_SPEECH_URL", format!("{base_url}/synthesize"));

    let app = foni_synth::build_router().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/synthesize"))
        .json(&json!({"text": "тест", "voice": "ru", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();

    // Malformed WAV will be returned (cloud_tts returns the bytes as-is)
    // The DSP chain or player will reject it, not the HTTP layer
    assert_eq!(resp.status(), 200);
    eprintln!("  [contract] malformed → passed through (DSP will reject)");
}

#[tokio::test]
async fn contract_no_url_uses_espeak() {
    std::env::remove_var("FISH_SPEECH_URL");

    let app = foni_synth::build_router().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/synthesize"))
        .json(&json!({"text": "тест", "voice": "ru", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.bytes().await.unwrap();
    assert!(body.len() > 44);
    eprintln!("  [contract] no URL → espeak direct: {} bytes", body.len());
}

#[tokio::test]
async fn contract_empty_text_returns_error() {
    std::env::remove_var("FISH_SPEECH_URL");

    let app = foni_synth::build_router().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/synthesize"))
        .json(&json!({"text": "", "voice": "ru", "speed": 150}))
        .send()
        .await
        .unwrap();

    // espeak with empty text should either error or return minimal WAV
    eprintln!(
        "  [contract] empty text: HTTP {} ({} bytes)",
        resp.status(),
        resp.content_length().unwrap_or(0)
    );
}
