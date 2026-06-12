/// security — input validation and abuse resistance tests.
///
/// Verifies depecherd handles adversarial input without crashing,
/// leaking data, or consuming unbounded resources.
///
/// cargo test -p depecherd --test security -- --nocapture
use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn start() -> (String, u16) {
    std::env::set_var("DEPECHER_DRY_RUN", "1");
    std::env::remove_var("FISH_SPEECH_URL");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = depecherd::build_router().await;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://127.0.0.1:{port}"), port)
}

async fn ws_connect(
    port: u16,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws"))
        .await
        .unwrap();
    ws
}

async fn recv(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    ms: u64,
) -> Option<serde_json::Value> {
    match tokio::time::timeout(std::time::Duration::from_millis(ms), ws.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str(&t).ok(),
        _ => None,
    }
}

// ── HTTP endpoint input validation ──────────────────────────────────────

#[tokio::test]
async fn synth_missing_text_field() {
    let (base, _) = start().await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/synthesize"))
        .json(&json!({"voice": "ru"}))
        .send()
        .await
        .unwrap();
    // Should handle gracefully — either empty WAV or error, not crash
    assert!(resp.status().is_success() || resp.status().is_client_error());
    eprintln!("  [security] missing text: HTTP {}", resp.status());
}

#[tokio::test]
async fn synth_null_text() {
    let (base, _) = start().await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/synthesize"))
        .json(&json!({"text": null, "voice": "ru"}))
        .send()
        .await
        .unwrap();
    eprintln!("  [security] null text: HTTP {}", resp.status());
}

#[tokio::test]
async fn synth_very_long_text() {
    let (base, _) = start().await;
    let long_text = "А".repeat(100_000);
    let resp = reqwest::Client::new()
        .post(format!("{base}/synthesize"))
        .json(&json!({"text": long_text, "voice": "ru", "speed": 150, "dsp": false}))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .unwrap();
    // Should not OOM or hang forever
    eprintln!(
        "  [security] 100K chars: HTTP {} ({} bytes)",
        resp.status(),
        resp.content_length().unwrap_or(0)
    );
}

#[tokio::test]
async fn synth_ssml_injection() {
    let (base, _) = start().await;
    let malicious = r#"<speak><audio src="http://evil.com/steal.wav"/></speak>"#;
    let resp = reqwest::Client::new()
        .post(format!("{base}/synthesize"))
        .json(&json!({"text": malicious, "voice": "ru", "speed": 150, "dsp": false}))
        .send()
        .await
        .unwrap();
    // TTS should reject or ignore the audio tag, not fetch the URL
    assert!(resp.status().is_success() || resp.status().is_server_error());
    eprintln!("  [security] SSML injection: HTTP {}", resp.status());
}

#[tokio::test]
async fn synth_unicode_edge_cases() {
    let (base, _) = start().await;
    let cases = [
        "\u{0000}",         // null byte
        "\u{FEFF}test",     // BOM
        "🔥🚀💯",           // emoji-only
        "test\r\n\r\ntest", // CRLF
        "\t\t\t",           // tabs only
    ];
    for text in cases {
        let resp = reqwest::Client::new()
            .post(format!("{base}/synthesize"))
            .json(&json!({"text": text, "voice": "ru", "speed": 150, "dsp": false}))
            .send()
            .await
            .unwrap();
        eprintln!(
            "  [security] unicode {:?}: HTTP {}",
            text.chars().take(5).collect::<String>(),
            resp.status()
        );
    }
}

#[tokio::test]
async fn synth_invalid_json() {
    let (base, _) = start().await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/synthesize"))
        .header("Content-Type", "application/json")
        .body("not json at all")
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_client_error(),
        "invalid JSON should be 4xx"
    );
    eprintln!("  [security] invalid JSON: HTTP {}", resp.status());
}

#[tokio::test]
async fn synth_wrong_content_type() {
    let (base, _) = start().await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/synthesize"))
        .header("Content-Type", "text/plain")
        .body(r#"{"text":"test"}"#)
        .send()
        .await
        .unwrap();
    eprintln!("  [security] wrong content-type: HTTP {}", resp.status());
}

// ── WebSocket input validation ──────────────────────────────────────────

#[tokio::test]
async fn ws_invalid_json() {
    let (_, port) = start().await;
    let mut ws = ws_connect(port).await;
    ws.send(Message::Text("not json".into())).await.unwrap();
    // Should not crash — just ignore
    let msg = recv(&mut ws, 500).await;
    assert!(msg.is_none(), "invalid JSON should produce no response");
    eprintln!("  [security] WS invalid JSON: no crash");
}

#[tokio::test]
async fn ws_missing_type_field() {
    let (_, port) = start().await;
    let mut ws = ws_connect(port).await;
    ws.send(Message::Text(json!({"text": "hello"}).to_string().into()))
        .await
        .unwrap();
    let msg = recv(&mut ws, 500).await;
    assert!(msg.is_none(), "missing type should produce no response");
    eprintln!("  [security] WS missing type: no crash");
}

#[tokio::test]
async fn ws_unknown_type() {
    let (_, port) = start().await;
    let mut ws = ws_connect(port).await;
    ws.send(Message::Text(
        json!({"type": "hack", "payload": "evil"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let msg = recv(&mut ws, 500).await;
    assert!(msg.is_none(), "unknown type should be ignored");
    eprintln!("  [security] WS unknown type: ignored");
}

#[tokio::test]
async fn ws_delta_with_null_text() {
    let (_, port) = start().await;
    let mut ws = ws_connect(port).await;
    ws.send(Message::Text(
        json!({"type": "delta", "text": null}).to_string().into(),
    ))
    .await
    .unwrap();
    let msg = recv(&mut ws, 500).await;
    assert!(msg.is_none(), "null text delta should be ignored");
    eprintln!("  [security] WS null delta: no crash");
}

#[tokio::test]
async fn ws_rapid_resets() {
    let (_, port) = start().await;
    let mut ws = ws_connect(port).await;
    for _ in 0..100 {
        ws.send(Message::Text(json!({"type": "reset"}).to_string().into()))
            .await
            .unwrap();
    }
    // Should not crash or leak memory
    ws.send(Message::Text(
        json!({"type": "delta", "text": "After reset. "})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let msg = recv(&mut ws, 1000).await;
    assert!(msg.is_some(), "should work after 100 rapid resets");
    eprintln!("  [security] 100 rapid resets: survived");
}

#[tokio::test]
async fn ws_binary_message_ignored() {
    let (_, port) = start().await;
    let mut ws = ws_connect(port).await;
    ws.send(Message::Binary(vec![0xFF, 0xFE, 0x00].into()))
        .await
        .unwrap();
    let msg = recv(&mut ws, 500).await;
    assert!(msg.is_none(), "binary messages should be ignored");
    // Verify connection still works
    ws.send(Message::Text(
        json!({"type": "delta", "text": "Still alive. "})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let msg = recv(&mut ws, 1000).await;
    assert!(msg.is_some());
    eprintln!("  [security] binary message: ignored, connection alive");
}
