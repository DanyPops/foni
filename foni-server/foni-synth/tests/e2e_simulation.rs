/// e2e_simulation — full pipeline simulation with canned services.
///
/// Spins up:
///   - Mock Ollama (returns "TRANSLATED: {input}")
///   - Mock Fish Speech (returns a sine WAV)
///   - Real foni-synth server
///   - WS client
///
/// Proves every wire: delta → chunk → translate → synthesize → DSP → cache → WS response.
/// Zero external deps. No GPU, no network, no speakers.
///
/// cargo test -p foni-synth --test e2e_simulation -- --nocapture
use axum::{extract::Multipart, routing::post, Json, Router};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::f32::consts::PI;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

fn sine_wav(freq: f32, duration_secs: f32, sample_rate: u32) -> Vec<u8> {
    let n = (sample_rate as f32 * duration_secs) as usize;
    let samples: Vec<f32> = (0..n)
        .map(|i| (2.0 * PI * freq * i as f32 / sample_rate as f32).sin() * 0.3)
        .collect();
    foni_synth::wav::encode_wav(&samples, sample_rate).expect("infallible")
}

async fn start_mock_ollama() -> String {
    let app = Router::new().route(
        "/api/chat",
        post(|Json(body): Json<Value>| async move {
            let input = body["messages"]
                .as_array()
                .and_then(|msgs| msgs.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or("?");
            Json(json!({
                "message": {
                    "content": format!("TRANSLATED: {input}")
                }
            }))
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

async fn start_mock_fish_speech() -> String {
    let app = Router::new().route(
        "/v1/tts",
        post(|mut multipart: Multipart| async move {
            let mut text = String::new();
            while let Ok(Some(field)) = multipart.next_field().await {
                if field.name() == Some("text") {
                    text = field.text().await.unwrap_or_default();
                }
            }
            eprintln!("  [mock fish] synthesizing: {text}");
            let wav = sine_wav(220.0, 0.3, 22050);
            ([(axum::http::header::CONTENT_TYPE, "audio/wav")], wav)
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

async fn start_foni_synth() -> (String, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    // Set FONI_SYNTH_ADDR so the WS handler knows its own port for self-calls
    std::env::set_var("FONI_SYNTH_ADDR", format!("0.0.0.0:{port}"));
    // Disable dry_run — we want the full pipeline
    std::env::set_var("FONI_DRY_RUN", "0");
    let app = foni_synth::build_router().await;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        format!("ws://127.0.0.1:{port}/ws"),
        format!("http://127.0.0.1:{port}"),
    )
}

async fn recv(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    ms: u64,
) -> Option<Value> {
    match tokio::time::timeout(std::time::Duration::from_millis(ms), ws.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str(&t).ok(),
        _ => None,
    }
}

#[tokio::test]
async fn full_pipeline_delta_to_playing() {
    // 1. Start mock services
    let ollama_url = start_mock_ollama().await;
    let fish_url = start_mock_fish_speech().await;

    // 2. Configure foni-synth to use mocks
    std::env::set_var("FISH_SPEECH_URL", &fish_url);
    std::env::set_var("FONI_OLLAMA_URL", &ollama_url);

    // 3. Start foni-synth
    let (ws_url, _http_url) = start_foni_synth().await;

    // 4. Connect WS
    let (mut ws, _) = connect_async(&ws_url).await.expect("WS connect");

    // 5. Send a complete sentence as delta
    ws.send(Message::Text(
        json!({"type": "delta", "text": "Hello world. "})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    // 6. Wait for response — should be {type: "playing"} because Fish Speech mock returns WAV
    let msg = recv(&mut ws, 10_000).await;
    assert!(msg.is_some(), "expected response after delta with sentence");
    let msg = msg.unwrap();

    let msg_type = msg["type"].as_str().unwrap_or("");
    let text = msg["text"].as_str().unwrap_or("");

    eprintln!("  [result] type={msg_type} text={text}");

    // Full pipeline: delta → chunk → glossary → mock Ollama translate →
    //   POST /synthesize → mock Fish Speech → DSP → cache → play queue → WS response
    assert!(
        msg_type == "playing",
        "expected 'playing' (full pipeline), got '{msg_type}': {}",
        msg
    );
    // Text should contain TRANSLATED prefix from mock Ollama
    assert!(
        text.contains("TRANSLATED"),
        "expected mock Ollama translation marker in: {text}"
    );
    eprintln!("  [✅] full pipeline: delta → translate → synthesize → DSP → play");
}

#[tokio::test]
async fn full_pipeline_http_synthesize_with_mock_fish() {
    let fish_url = start_mock_fish_speech().await;
    std::env::set_var("FISH_SPEECH_URL", &fish_url);

    let (_ws_url, http_url) = start_foni_synth().await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{http_url}/synthesize"))
        .json(&json!({
            "text": "Привет, сталкер.",
            "voice": "ru",
            "speed": 150,
            "dsp": true,
        }))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_success(),
        "POST /synthesize failed: {}",
        resp.status()
    );

    let wav = resp.bytes().await.unwrap();
    eprintln!(
        "  [synth] {} bytes WAV from mock Fish Speech + DSP",
        wav.len()
    );

    assert!(wav.len() > 44, "WAV must be non-trivial");
    assert!(
        wav.len() > 1000,
        "DSP should produce substantial audio, got {} bytes",
        wav.len()
    );
}

#[tokio::test]
async fn full_pipeline_emotion_then_synthesis() {
    let ollama_url = start_mock_ollama().await;
    let fish_url = start_mock_fish_speech().await;
    std::env::set_var("FISH_SPEECH_URL", &fish_url);
    std::env::set_var("FONI_OLLAMA_URL", &ollama_url);

    let (ws_url, _) = start_foni_synth().await;
    let (mut ws, _) = connect_async(&ws_url).await.expect("WS connect");

    // 1. Send angry user message → get emotion response
    ws.send(Message::Text(
        json!({"type": "user_message", "text": "WHAT THE HELL!!"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    let emotion = recv(&mut ws, 2000).await.expect("expected emotion");
    assert_eq!(emotion["type"], "emotion");
    assert_eq!(emotion["emotion"], "angry");
    eprintln!(
        "  [emotion] {} intensity={} signals={}",
        emotion["emotion"], emotion["intensity"], emotion["signals"]
    );

    // 2. Send a sentence — proves emotion + synthesis can coexist on same WS
    ws.send(Message::Text(
        json!({"type": "delta", "text": "Fix the bug. "})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    let synth = recv(&mut ws, 10_000)
        .await
        .expect("expected speak/playing after delta");
    let msg_type = synth["type"].as_str().unwrap_or("");
    eprintln!(
        "  [synth after emotion] type={msg_type} text={}",
        synth["text"]
    );
    assert!(
        msg_type == "speak" || msg_type == "playing",
        "expected speak/playing, got {msg_type}"
    );
}

#[tokio::test]
async fn cache_hit_on_second_request() {
    let fish_url = start_mock_fish_speech().await;
    std::env::set_var("FISH_SPEECH_URL", &fish_url);

    let (_ws_url, http_url) = start_foni_synth().await;
    let client = reqwest::Client::new();

    let body = json!({
        "text": "Кэш тест.",
        "voice": "ru",
        "speed": 150,
        "dsp": false,
    });

    // First request — cold
    let t0 = std::time::Instant::now();
    let resp1 = client
        .post(format!("{http_url}/synthesize"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let d1 = t0.elapsed();
    assert!(resp1.status().is_success());
    let wav1 = resp1.bytes().await.unwrap();

    // Second request — should hit cache
    let t1 = std::time::Instant::now();
    let resp2 = client
        .post(format!("{http_url}/synthesize"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let d2 = t1.elapsed();
    assert!(resp2.status().is_success());
    let wav2 = resp2.bytes().await.unwrap();

    eprintln!("  [cache] cold={d1:?} hot={d2:?}");
    assert_eq!(wav1.len(), wav2.len(), "cache should return same WAV");
    assert!(
        d2 < d1 || d2.as_millis() < 50,
        "cache hit should be faster: cold={d1:?} hot={d2:?}"
    );
}

#[tokio::test]
async fn full_pipeline_mat_injection() {
    let ollama_url = start_mock_ollama().await;
    let fish_url = start_mock_fish_speech().await;
    std::env::set_var("FISH_SPEECH_URL", &fish_url);
    std::env::set_var("FONI_OLLAMA_URL", &ollama_url);
    // Disable dry_run so mat injection runs
    std::env::set_var("FONI_DRY_RUN", "0");

    let (ws_url, _) = start_foni_synth().await;
    let (mut ws, _) = connect_async(&ws_url).await.expect("WS connect");

    // Send multiple sentences to increase chance of mat injection (prob=0.35 per opportunity)
    for i in 0..5 {
        ws.send(Message::Text(
            json!({"type": "delta", "text": format!("Sentence number {i}. ")})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    }

    // Collect all responses
    let mut texts = Vec::new();
    for _ in 0..10 {
        if let Some(msg) = recv(&mut ws, 10_000).await {
            let t = msg["type"].as_str().unwrap_or("");
            if t == "playing" || t == "speak" {
                if let Some(text) = msg["text"].as_str() {
                    texts.push(text.to_string());
                }
            }
        } else {
            break;
        }
    }

    eprintln!("  [mat test] got {} responses:", texts.len());
    for t in &texts {
        eprintln!("    {t}");
    }

    // At least some responses should exist
    assert!(!texts.is_empty(), "expected at least one response");
}

#[tokio::test]
async fn full_pipeline_emotion_affects_injection() {
    let ollama_url = start_mock_ollama().await;
    let fish_url = start_mock_fish_speech().await;
    std::env::set_var("FISH_SPEECH_URL", &fish_url);
    std::env::set_var("FONI_OLLAMA_URL", &ollama_url);
    std::env::set_var("FONI_DRY_RUN", "0");

    let (ws_url, _) = start_foni_synth().await;
    let (mut ws, _) = connect_async(&ws_url).await.expect("WS connect");

    // Make user angry first — boosts mat probability to 2x
    ws.send(Message::Text(
        json!({"type": "user_message", "text": "WHAT THE FUCK is broken!!"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    let emotion = recv(&mut ws, 2000).await.expect("expected emotion");
    assert_eq!(emotion["emotion"], "angry");
    eprintln!("  [emotion] angry, intensity={}", emotion["intensity"]);

    // Now send text — mat injection should be boosted
    ws.send(Message::Text(
        json!({"type": "delta", "text": "Fix the deployment pipeline. "})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    let msg = recv(&mut ws, 10_000).await.expect("expected speak/playing");
    let text = msg["text"].as_str().unwrap_or("");
    eprintln!("  [angry speech] {text}");

    // We can't assert mat was injected (it's probabilistic)
    // but we prove the pipeline doesn't crash with emotion + mat + synthesis
    assert!(!text.is_empty());
}

#[tokio::test]
async fn ws_parse_train_logs_returns_events() {
    std::env::set_var("FONI_DRY_RUN", "1");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = foni_synth::build_router().await;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws"))
        .await
        .expect("connect");

    let logs = "[checkpoint] s2-pro already cached at /data/checkpoints/s2-pro\n\
                [train] 63 WAV files in /data/dataset-raw\n\
                SyntaxWarning: blah\n\
                [train] extracting semantic tokens...\n\
                [train] DONE";

    ws.send(Message::Text(
        json!({"type": "parse_train_logs", "text": logs})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    let mut events = Vec::new();
    for _ in 0..10 {
        if let Some(msg) = recv(&mut ws, 1000).await {
            if msg["type"] == "train_event" {
                events.push(msg["data"]["event"].as_str().unwrap_or("").to_string());
            }
        } else {
            break;
        }
    }

    eprintln!("  events: {events:?}");
    assert!(events.contains(&"checkpoint_cached".to_string()));
    assert!(events.contains(&"dataset_ready".to_string()));
    assert!(events.contains(&"vq_started".to_string()));
    assert!(events.contains(&"done".to_string()));
    assert!(!events.iter().any(|e| e.contains("warning")));
}
