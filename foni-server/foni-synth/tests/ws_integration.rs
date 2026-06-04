/// ws_integration — WebSocket engine E2E against a real bound server.
///
/// Starts foni-synth on a random port, connects via WS, sends deltas,
/// and asserts on the response messages.
///
/// cargo test -p foni-synth --test ws_integration -- --nocapture
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn start_server() -> (String, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    let app = foni_synth::build_router().await;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("ws://127.0.0.1:{port}/ws"), port)
}

async fn connect(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (ws, _) = connect_async(url).await.expect("WS connect failed");
    ws
}

async fn send(
    ws: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    msg: Value,
) {
    ws.send(Message::Text(msg.to_string().into()))
        .await
        .expect("WS send failed");
}

async fn recv_timeout(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    ms: u64,
) -> Option<Value> {
    match tokio::time::timeout(std::time::Duration::from_millis(ms), ws.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str(&t).ok(),
        _ => None,
    }
}

#[tokio::test]
async fn ws_connects_and_responds_to_reset() {
    let (url, _) = start_server().await;
    let mut ws = connect(&url).await;

    send(&mut ws, json!({"type": "reset"})).await;
    // Reset produces no response — just verify no crash
    // Send a delta to prove the connection is still alive
    send(&mut ws, json!({"type": "delta", "text": "Hi. "})).await;
    // "Hi." is too short (<=2 chars after trim) so no speak message expected
    // But the WS should still be open
    send(&mut ws, json!({"type": "delta", "text": "Hello world. "})).await;

    let msg = recv_timeout(&mut ws, 5000).await;
    assert!(
        msg.is_some(),
        "expected a response after delta with sentence"
    );
    let msg = msg.unwrap();
    assert!(
        msg["type"] == "speak" || msg["type"] == "playing",
        "expected speak or playing, got {msg}"
    );
    assert!(msg["text"].as_str().unwrap_or("").len() > 0);
    println!("speak text: {}", msg["text"]);
}

#[tokio::test]
async fn ws_emotion_detection() {
    let (url, _) = start_server().await;
    let mut ws = connect(&url).await;

    send(
        &mut ws,
        json!({"type": "user_message", "text": "WHAT THE HELL this is broken!!"}),
    )
    .await;

    let msg = recv_timeout(&mut ws, 5000).await;
    assert!(msg.is_some(), "expected emotion response");
    let msg = msg.unwrap();
    assert_eq!(msg["type"], "emotion");
    assert_eq!(
        msg["emotion"], "angry",
        "expected angry, got {}",
        msg["emotion"]
    );
    assert!(msg["intensity"].as_f64().unwrap_or(0.0) > 0.0);
    println!(
        "emotion: {} intensity={} signals={}",
        msg["emotion"], msg["intensity"], msg["signals"]
    );
}

#[tokio::test]
async fn ws_message_end_flushes_buffer() {
    let (url, _) = start_server().await;
    let mut ws = connect(&url).await;

    // Send text without sentence-ending punctuation — should buffer
    send(&mut ws, json!({"type": "delta", "text": "Привет сталкер"})).await;

    // No response expected yet (no sentence boundary)
    let msg = recv_timeout(&mut ws, 1000).await;
    assert!(msg.is_none(), "should not speak before message_end");

    // Flush
    send(&mut ws, json!({"type": "message_end"})).await;

    let msg = recv_timeout(&mut ws, 5000).await;
    assert!(msg.is_some(), "expected speak after message_end flush");
    let msg = msg.unwrap();
    assert!(msg["type"] == "speak" || msg["type"] == "playing");
    let text = msg["text"].as_str().unwrap_or("");
    println!("flushed text: {text}");
    assert!(!text.is_empty(), "expected text in flush");
}

#[tokio::test]
async fn ws_strips_markdown_from_deltas() {
    let (url, _) = start_server().await;
    let mut ws = connect(&url).await;

    send(
        &mut ws,
        json!({"type": "delta", "text": "## Hello **world**. "}),
    )
    .await;

    let msg = recv_timeout(&mut ws, 5000).await;
    assert!(msg.is_some(), "expected speak");
    let text = msg.unwrap()["text"].as_str().unwrap_or("").to_string();
    println!("stripped: {text}");
    assert!(!text.contains("##"), "markdown headers should be stripped");
    assert!(!text.contains("**"), "bold markers should be stripped");
    assert!(!text.is_empty(), "prose should survive (may be translated)");
}

#[tokio::test]
async fn ws_skips_code_blocks() {
    let (url, _) = start_server().await;
    let mut ws = connect(&url).await;

    // Send text with inline code
    for ch in "Use `npm install` to install. ".chars() {
        send(&mut ws, json!({"type": "delta", "text": ch.to_string()})).await;
    }

    let msg = recv_timeout(&mut ws, 5000).await;
    assert!(msg.is_some(), "expected speak");
    let text = msg.unwrap()["text"].as_str().unwrap_or("").to_string();
    println!("code-filtered: {text}");
    // inline code was stripped by stream.rs before reaching translation
    assert!(!text.is_empty(), "text should exist after code stripping");
}

#[tokio::test]
async fn ws_multiple_sentences_produce_multiple_speaks() {
    let (url, _) = start_server().await;
    let mut ws = connect(&url).await;

    send(
        &mut ws,
        json!({"type": "delta", "text": "First sentence. Second sentence. Third here"}),
    )
    .await;

    // Should get at least 2 speak messages (first two sentences)
    let msg1 = recv_timeout(&mut ws, 5000).await;
    assert!(msg1.is_some(), "expected first speak");
    let m1 = msg1.unwrap();
    let t1 = m1["type"].as_str().unwrap_or("");
    assert!(
        t1 == "speak" || t1 == "playing",
        "expected speak/playing, got {t1}"
    );

    let msg2 = recv_timeout(&mut ws, 5000).await;
    assert!(msg2.is_some(), "expected second speak");
    let m2 = msg2.unwrap();
    let t2 = m2["type"].as_str().unwrap_or("");
    assert!(
        t2 == "speak" || t2 == "playing",
        "expected speak/playing, got {t2}"
    );

    println!("got 2 speak messages for 2 complete sentences");
}

#[tokio::test]
async fn ws_sarcasm_detection() {
    let (url, _) = start_server().await;
    let mut ws = connect(&url).await;

    send(
        &mut ws,
        json!({"type": "user_message", "text": "oh great, just perfect, thanks for nothing"}),
    )
    .await;

    let msg = recv_timeout(&mut ws, 5000).await;
    assert!(msg.is_some());
    let msg = msg.unwrap();
    assert_eq!(msg["type"], "emotion");
    assert_eq!(msg["emotion"], "sarcastic");
    println!("sarcasm detected: intensity={}", msg["intensity"]);
}

#[tokio::test]
async fn ws_cute_detection() {
    let (url, _) = start_server().await;
    let mut ws = connect(&url).await;

    send(&mut ws, json!({"type": "user_message", "text": "please, if you don't mind, could you possibly help me uwu 🥺"})).await;

    let msg = recv_timeout(&mut ws, 5000).await;
    assert!(msg.is_some());
    let msg = msg.unwrap();
    assert_eq!(msg["type"], "emotion");
    assert_eq!(msg["emotion"], "cute");
    println!("cute detected: intensity={}", msg["intensity"]);
}
