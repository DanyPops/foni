/// ws_integration — WebSocket engine E2E against a real server in dry_run mode.
///
/// No external dependencies: Ollama, espeak-ng, and paplay are all skipped.
/// Tests prove the full pipeline logic: delta → chunk → strip → translate → speak.
///
/// cargo test -p foni-synth --test ws_integration -- --nocapture
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn start_server() -> String {
    // Force dry_run so no Ollama/espeak/paplay calls
    std::env::set_var("FONI_DRY_RUN", "1");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = foni_synth::build_router().await;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("ws://127.0.0.1:{}/ws", addr.port())
}

async fn connect(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (ws, _) = connect_async(url).await.expect("WS connect failed");
    ws
}

async fn send_msg(
    ws: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    msg: Value,
) {
    ws.send(Message::Text(msg.to_string().into()))
        .await
        .expect("WS send failed");
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

// ── Stream chunking ─────────────────────────────────────────────────────────

#[tokio::test]
async fn delta_with_sentence_produces_speak() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(&mut ws, json!({"type": "delta", "text": "Hello world. "})).await;

    let msg = recv(&mut ws, 1000).await;
    assert!(msg.is_some(), "expected speak after complete sentence");
    let msg = msg.unwrap();
    assert_eq!(msg["type"], "speak");
    let text = msg["text"].as_str().unwrap_or("");
    assert!(!text.is_empty());
    println!("speak: {text}");
}

#[tokio::test]
async fn mid_sentence_buffers_until_boundary() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(&mut ws, json!({"type": "delta", "text": "Hello there"})).await;

    let msg = recv(&mut ws, 500).await;
    assert!(msg.is_none(), "no speak without sentence boundary");
}

#[tokio::test]
async fn message_end_flushes_buffer() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(&mut ws, json!({"type": "delta", "text": "Buffered text"})).await;
    send_msg(&mut ws, json!({"type": "message_end"})).await;

    let msg = recv(&mut ws, 1000).await;
    assert!(msg.is_some(), "expected flush on message_end");
    let val = msg.unwrap();
    let text = val["text"].as_str().unwrap_or("");
    assert!(text.contains("Buffered text"));
    println!("flushed: {text}");
}

#[tokio::test]
async fn multiple_sentences_produce_multiple_speaks() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "First. Second. Third"}),
    )
    .await;

    let m1 = recv(&mut ws, 1000).await;
    let m2 = recv(&mut ws, 1000).await;
    assert!(m1.is_some(), "expected first speak");
    assert!(m2.is_some(), "expected second speak");
    println!("2 speaks for 2 complete sentences");
}

#[tokio::test]
async fn reset_clears_stream_state() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(&mut ws, json!({"type": "delta", "text": "Partial buffer"})).await;
    send_msg(&mut ws, json!({"type": "reset"})).await;
    // Buffer was cleared — message_end should have nothing to flush
    send_msg(&mut ws, json!({"type": "message_end"})).await;

    let msg = recv(&mut ws, 500).await;
    assert!(msg.is_none(), "reset should clear the buffer");
}

// ── Markdown stripping ──────────────────────────────────────────────────────

#[tokio::test]
async fn strips_markdown_headers_and_bold() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "## Hello **world**. "}),
    )
    .await;

    let msg = recv(&mut ws, 1000).await.expect("expected speak");
    let text = msg["text"].as_str().unwrap_or("");
    assert!(!text.contains("##"), "headers stripped");
    assert!(!text.contains("**"), "bold stripped");
    assert!(text.contains("Hello") && text.contains("world"));
    println!("stripped: {text}");
}

#[tokio::test]
async fn code_blocks_filtered_from_speech() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    for ch in "Use `npm install` to set up. ".chars() {
        send_msg(&mut ws, json!({"type": "delta", "text": ch.to_string()})).await;
    }

    let msg = recv(&mut ws, 1000).await.expect("expected speak");
    let text = msg["text"].as_str().unwrap_or("");
    assert!(!text.contains("npm install"), "code stripped");
    assert!(text.contains("Use") || text.contains("set up"));
    println!("code-filtered: {text}");
}

// ── IT glossary ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn glossary_replaces_it_terms() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "Deploy the server. "}),
    )
    .await;

    let msg = recv(&mut ws, 1000).await.expect("expected speak");
    let text = msg["text"].as_str().unwrap_or("");
    assert!(
        text.contains("деплой") || text.contains("сервер"),
        "IT terms should be replaced: {text}"
    );
    println!("glossary: {text}");
}

// ── Emotion detection ───────────────────────────────────────────────────────

#[tokio::test]
async fn detects_angry_emotion() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "user_message", "text": "WHAT THE HELL this is broken!!"}),
    )
    .await;

    let msg = recv(&mut ws, 1000).await.expect("expected emotion");
    assert_eq!(msg["type"], "emotion");
    assert_eq!(msg["emotion"], "angry");
    assert!(msg["intensity"].as_f64().unwrap_or(0.0) > 0.0);
    println!(
        "angry: intensity={} signals={}",
        msg["intensity"], msg["signals"]
    );
}

#[tokio::test]
async fn detects_sarcastic_emotion() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "user_message", "text": "oh great, just perfect, thanks for nothing"}),
    )
    .await;

    let msg = recv(&mut ws, 1000).await.expect("expected emotion");
    assert_eq!(msg["emotion"], "sarcastic");
}

#[tokio::test]
async fn detects_cute_emotion() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "user_message", "text": "please help uwu 🥺"}),
    )
    .await;

    let msg = recv(&mut ws, 1000).await.expect("expected emotion");
    assert_eq!(msg["emotion"], "cute");
}

#[tokio::test]
async fn detects_excited_emotion() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "user_message", "text": "this is amazing!!! 🔥🚀"}),
    )
    .await;

    let msg = recv(&mut ws, 1000).await.expect("expected emotion");
    assert_eq!(msg["emotion"], "excited");
}

#[tokio::test]
async fn detects_frustrated_emotion() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "user_message", "text": "ugh, not again... seriously??"}),
    )
    .await;

    let msg = recv(&mut ws, 1000).await.expect("expected emotion");
    assert_eq!(msg["emotion"], "frustrated");
}

#[tokio::test]
async fn neutral_text_returns_neutral() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "user_message", "text": "Can you refactor the config module?"}),
    )
    .await;

    let msg = recv(&mut ws, 1000).await.expect("expected emotion");
    assert_eq!(msg["emotion"], "neutral");
    assert_eq!(msg["intensity"], 0.0);
}
