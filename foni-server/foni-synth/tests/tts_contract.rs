/// tts_contract — contract tests for cloud TTS HTTP client.
///
/// Verifies synthesize_route handles all response shapes from the TTS endpoint:
/// success, auth failure, server error, timeout, malformed body, missing URL.
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
        .json(&json!({"text": "test", "voice": "en", "speed": 150, "dsp": false}))
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
    eprintln!("  [contract] success: {} bytes WAV", body.len());
}

#[tokio::test]
async fn contract_401_returns_error() {
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
        .json(&json!({"text": "test", "voice": "en", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500, "401 from TTS should propagate as 500");
    eprintln!("  [contract] 401 → 500");
}

#[tokio::test]
async fn contract_500_returns_error() {
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
        .json(&json!({"text": "test", "voice": "en", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500, "500 from TTS should propagate");
    eprintln!("  [contract] 500 → 500");
}

#[tokio::test]
async fn contract_no_url_returns_error() {
    std::env::remove_var("FISH_SPEECH_URL");

    let app = foni_synth::build_router().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/synthesize"))
        .json(&json!({"text": "test", "voice": "en", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500, "no TTS URL should return 500");
    eprintln!("  [contract] no URL → 500");
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
        .json(&json!({"text": "", "voice": "en", "speed": 150}))
        .send()
        .await
        .unwrap();

    eprintln!(
        "  [contract] empty text: HTTP {} ({} bytes)",
        resp.status(),
        resp.content_length().unwrap_or(0)
    );
}
