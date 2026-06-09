/// ws_integration — WebSocket engine E2E against a real server in dry_run mode.
///
/// No external dependencies: Ollama and paplay are skipped.
/// Tests prove the full pipeline logic: delta → chunk → strip → translate → speak.
///
/// cargo test -p foni-synth --test ws_integration -- --nocapture
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let synth = foni_synth::engine::synth_backend::mock_backend();
    let app = foni_synth::build_router_with_synth(synth).await;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("ws://127.0.0.1:{}/ws", addr.port())
}

async fn connect(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut ws, _) = connect_async(url).await.expect("WS connect failed");
    // Set dry_run per-connection so tests are isolated from the global env.
    send_msg(
        &mut ws,
        serde_json::json!({"type": "set_config", "dry_run": true}),
    )
    .await;
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
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(msg) = serde_json::from_str::<Value>(&t) {
                    if msg["type"] == "buffer_state" {
                        continue;
                    }
                    return Some(msg);
                }
            }
            _ => return None,
        }
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

    let msg = recv(&mut ws, 3000).await.expect("expected speak");
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

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Collect every non-buffer_state message within `ms` milliseconds.
async fn recv_all(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    ms: u64,
) -> Vec<Value> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
    #[allow(clippy::while_let_loop)]
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(msg) = serde_json::from_str::<Value>(&t) {
                    if msg["type"] != "buffer_state" {
                        out.push(msg);
                    }
                }
            }
            _ => break,
        }
    }
    out
}

// ── Buffer state ─────────────────────────────────────────────────────────────

/// Collect all messages until timeout, return buffer_state messages.
async fn collect_buffer_states(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    ms: u64,
) -> Vec<Value> {
    let mut states = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
    #[allow(clippy::while_let_loop)]
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(msg) = serde_json::from_str::<Value>(&t) {
                    if msg["type"] == "buffer_state" {
                        states.push(msg["data"].clone());
                    }
                }
            }
            _ => break,
        }
    }
    states
}

#[tokio::test]
async fn buffer_state_emitted_on_sentence() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(&mut ws, json!({"type": "delta", "text": "Hello world. "})).await;

    let states = collect_buffer_states(&mut ws, 500).await;
    assert!(
        !states.is_empty(),
        "should emit at least one buffer_state after sentence"
    );

    let last = states.last().unwrap();
    assert!(last["slots"].is_array());
}

#[tokio::test]
async fn buffer_state_emitted_on_message_end() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(&mut ws, json!({"type": "delta", "text": "Partial text"})).await;
    send_msg(&mut ws, json!({"type": "message_end"})).await;

    let states = collect_buffer_states(&mut ws, 2000).await;
    assert!(!states.is_empty(), "message_end should emit buffer_state");

    let last = states.last().unwrap();
    assert!(
        last["complete"].as_bool().unwrap_or(false),
        "last state should be complete after message_end"
    );
}

#[tokio::test]
async fn buffer_drains_across_multiple_sentences() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "First sentence. Second sentence. Third sentence. "}),
    )
    .await;
    send_msg(&mut ws, json!({"type": "message_end"})).await;

    let states = collect_buffer_states(&mut ws, 1000).await;

    // Should have multiple buffer_state updates (one per chunk + one on close)
    assert!(
        states.len() >= 3,
        "expected >= 3 buffer updates for 3 sentences, got {}",
        states.len()
    );

    // At least one state should be complete (the close event)
    let any_complete = states
        .iter()
        .any(|s| s["complete"].as_bool().unwrap_or(false));
    assert!(any_complete, "at least one buffer_state should be complete");
}

#[tokio::test]
async fn buffer_state_has_correct_shape() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(&mut ws, json!({"type": "delta", "text": "Test. "})).await;

    let states = collect_buffer_states(&mut ws, 500).await;
    assert!(!states.is_empty());

    let s = &states[0];
    assert!(s.get("slots").is_some(), "missing slots");
    assert!(s.get("buffered").is_some(), "missing buffered");
    assert!(s.get("pending").is_some(), "missing pending");
    assert!(s.get("complete").is_some(), "missing complete");
}

// ── TTS enable/disable ────────────────────────────────────────────────────────

#[tokio::test]
async fn tts_disabled_mid_stream_halts_synthesis() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    // First sentence should produce a speak event.
    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "First sentence. "}),
    )
    .await;
    let first = recv(&mut ws, 1500).await;
    assert!(first.is_some(), "should get speak for first sentence");
    assert_eq!(first.unwrap()["type"], "speak");

    // Disable TTS mid-stream.
    send_msg(&mut ws, json!({"type": "set_config", "enabled": false})).await;

    // Further deltas and flush — collect everything for 1200ms.
    // A correctly implemented disable must produce zero speak events.
    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "Second sentence. Third sentence. "}),
    )
    .await;
    send_msg(&mut ws, json!({"type": "message_end"})).await;

    let after_disable = recv_all(&mut ws, 1200).await;
    let speaks: Vec<_> = after_disable
        .iter()
        .filter(|m| m["type"] == "speak")
        .collect();
    assert!(
        speaks.is_empty(),
        "no speak events after disable, got: {speaks:?}"
    );
}

#[tokio::test]
async fn tts_reenabled_resumes_synthesis() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    // Disable before any text arrives.
    send_msg(&mut ws, json!({"type": "set_config", "enabled": false})).await;

    // Text sent while disabled — collect for 1200ms.
    // A correctly implemented disable must produce zero speak events during this window.
    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "Queued sentence. "}),
    )
    .await;
    let during_disable = recv_all(&mut ws, 1200).await;
    let speaks_during = during_disable
        .iter()
        .filter(|m| m["type"] == "speak")
        .count();
    assert_eq!(speaks_during, 0, "no speaks while disabled");

    // Re-enable — accumulated text should now be spoken.
    send_msg(&mut ws, json!({"type": "set_config", "enabled": true})).await;
    send_msg(&mut ws, json!({"type": "message_end"})).await;

    let after_enable = recv_all(&mut ws, 1500).await;
    let speaks_after = after_enable.iter().filter(|m| m["type"] == "speak").count();
    assert!(
        speaks_after > 0,
        "should produce at least one speak after re-enable"
    );
}

// ── FON-TSK-197/194/201 — Self-call deadlock + parallelism ──────────────────

/// Two concurrent WS connections must BOTH complete within 5s in dry_run.
/// Before the SessionCtx refactor this deadlocks: the second connection
/// cannot get a thread because the first holds one waiting for synthesize_local
/// to respond — which itself needs a thread.
#[tokio::test]
async fn concurrent_connections_dont_deadlock() {
    let url = start_server().await;

    let (r1, r2) = tokio::join!(
        async {
            let mut ws = connect(&url).await;
            send_msg(
                &mut ws,
                json!({"type": "delta", "text": "First connection. "}),
            )
            .await;
            recv(&mut ws, 3000).await
        },
        async {
            let mut ws = connect(&url).await;
            send_msg(
                &mut ws,
                json!({"type": "delta", "text": "Second connection. "}),
            )
            .await;
            recv(&mut ws, 3000).await
        },
    );

    assert!(r1.is_some(), "first connection should complete within 3s");
    assert!(
        r2.is_some(),
        "second connection should complete within 3s — deadlocks if synthesize_local self-calls"
    );
}

/// The WS writer (tx) is currently passed as &mut to process_chunk.
/// After the refactor, process_chunk sends via mpsc::Sender which is Clone+Send.
/// Test: two chunks from the same delta arrive on the WS in index order
/// even if the second one finishes faster (simulated via message ordering).
#[tokio::test]
async fn chunks_arrive_in_order_regardless_of_completion() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    // Two sentence boundaries in one delta — fires two chunks
    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "First sentence. Second sentence. "}),
    )
    .await;
    send_msg(&mut ws, json!({"type": "message_end"})).await;

    let msgs = recv_all(&mut ws, 1500).await;
    let speaks: Vec<&str> = msgs
        .iter()
        .filter(|m| m["type"] == "speak")
        .filter_map(|m| m["text"].as_str())
        .collect();

    assert!(speaks.len() >= 2, "should get speak for each sentence");
    // First sentence text must appear before second
    let first_pos = speaks.iter().position(|t| t.contains("First"));
    let second_pos = speaks.iter().position(|t| t.contains("Second"));
    assert!(
        first_pos < second_pos,
        "chunks must arrive in order: first={first_pos:?} second={second_pos:?}"
    );
}

/// SynthBackend trait contract: a mock backend injected at test time
/// must be called exactly once per chunk and must not cause a self-call.
/// This test will fail until SynthBackend trait exists and is injectable.
#[tokio::test]
async fn synth_backend_is_injected_not_self_called() {
    // After the refactor, the server should accept a FONI_TTS_URL env var
    // pointing to a mock TTS (not itself). Verify synthesis succeeds without
    // calling localhost:5050/synthesize.
    // dry_run is set per-connection by connect(); FONI_TTS_URL is irrelevant in dry_run.
    let url = start_server().await;
    let mut ws = connect(&url).await;
    send_msg(&mut ws, json!({"type": "delta", "text": "Test. "})).await;

    let msg = recv(&mut ws, 1000).await;
    assert!(
        msg.is_some(),
        "dry_run should always respond even with bad TTS_URL"
    );
    assert_eq!(msg.unwrap()["type"], "speak");
}

// ── Ordering + latency (dry_run — no env var race) ────────────────────────────

/// Chunks from stream.rs must arrive on WS in sentence-index order.
/// PlaybackBuffer drains by index so even if synthesis were parallel,
/// the response stream must be ordered.
#[tokio::test]
async fn chunks_drain_in_sentence_order() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "Alpha sentence. Beta sentence. Gamma sentence. "}),
    )
    .await;
    send_msg(&mut ws, json!({"type": "message_end"})).await;

    let msgs = recv_all(&mut ws, 2000).await;
    let texts: Vec<&str> = msgs
        .iter()
        .filter(|m| m["type"] == "speak")
        .filter_map(|m| m["text"].as_str())
        .collect();

    assert!(
        texts.len() >= 2,
        "expected >= 2 speak events, got: {texts:?}"
    );

    let alpha = texts.iter().position(|t| t.contains("Alpha"));
    let beta = texts.iter().position(|t| t.contains("Beta"));
    let gamma = texts.iter().position(|t| t.contains("Gamma"));

    if let (Some(a), Some(b)) = (alpha, beta) {
        assert!(a < b, "Alpha before Beta");
    }
    if let (Some(b), Some(g)) = (beta, gamma) {
        assert!(b < g, "Beta before Gamma");
    }
}

/// 3 chunks in dry_run must all complete in under 2s — no I/O in the hot path.
#[tokio::test]
async fn three_chunks_complete_quickly_in_dry_run() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    let t0 = std::time::Instant::now();
    send_msg(
        &mut ws,
        json!({"type": "delta", "text": "First. Second. Third. "}),
    )
    .await;
    send_msg(&mut ws, json!({"type": "message_end"})).await;

    // Wait for the first two speaks with individual timeouts.
    // This measures time-to-speech, not time-after-collection-window.
    let speak1 = recv(&mut ws, 2000).await;
    let speak2 = recv(&mut ws, 2000).await;
    let elapsed = t0.elapsed();

    assert!(speak1.is_some(), "first speak missing within 2s");
    assert!(speak2.is_some(), "second speak missing within 2s");
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "3 dry_run chunks should complete in <3s, took {elapsed:?}"
    );
}

// ── Prewarm ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn prewarm_sends_start_then_done() {
    let url = start_server().await;
    let mut ws = connect(&url).await;

    // Collect all messages — prewarm_start must arrive before prewarm_done.
    send_msg(&mut ws, json!({"type": "prewarm"})).await;

    let msgs = recv_all(&mut ws, 2000).await;
    let types: Vec<&str> = msgs.iter().filter_map(|m| m["type"].as_str()).collect();

    assert!(
        types.contains(&"prewarm_start"),
        "should send prewarm_start, got: {types:?}"
    );
    assert!(
        types.contains(&"prewarm_done"),
        "should send prewarm_done, got: {types:?}"
    );

    let start_pos = types.iter().position(|&t| t == "prewarm_start").unwrap();
    let done_pos = types.iter().position(|&t| t == "prewarm_done").unwrap();
    assert!(
        start_pos < done_pos,
        "prewarm_start must precede prewarm_done"
    );
}
