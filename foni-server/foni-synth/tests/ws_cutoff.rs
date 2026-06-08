//! ws_cutoff — E2E tests for Stop and Mute signal handling during active streams.
//!
//! Verifies that when a `reset` or `set_config {enabled:false}` arrives mid-stream:
//!   (a) no further synthesis events are emitted for the current turn, and
//!   (b) the play queue is drained immediately (no lingering audio).
//!
//! Two server modes used:
//!   dry_run=true   → pipeline-level tests; `speak` replaces `playing`; no real synthesis.
//!   MockSynthBackend → queue-level tests; real WS path, instant synthesis, no Modal calls.
//!
//! Known failures (document desired behaviour before fix):
//!   reset_drains_play_queue        — PlayQueue::stop() is currently a no-op.
//!   mute_drains_play_queue         — same root cause.

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

// ── Server fixtures ───────────────────────────────────────────────────────────

/// Start a server with a MockSynthBackend (instant sine WAV, no Modal calls).
/// dry_run is controlled per-connection via `set_config` — no env var needed.
async fn start_server() -> String {
    // Ensure any process-level dry_run flag is cleared so the server
    // starts in real-synthesis mode. Per-connection dry_run is set via WS.
    std::env::remove_var("FONI_DRY_RUN");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let synth = foni_synth::engine::synth_backend::mock_backend();
    let app = foni_synth::build_router_with_synth(synth).await;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("ws://127.0.0.1:{}/ws", addr.port())
}

/// Connect and immediately switch the connection to dry_run mode.
/// In dry_run: `speak` events replace real synthesis; play_queue unused.
async fn connect_dry(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut ws = connect(url).await;
    send(&mut ws, json!({"type": "set_config", "dry_run": true})).await;
    ws
}

/// Connect in real-synth mode (MockSynthBackend → instant WAV → `playing` events).
/// Sets lang=ru,ru to skip Ollama translation (same-lang → no translate call).
async fn connect_synth(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut ws = connect(url).await;
    // Disable translation so tests don’t hit Ollama.
    send(&mut ws, json!({"type": "set_config", "lang": "ru,ru"})).await;
    ws
}

async fn connect(
    url: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    connect_async(url).await.expect("WS connect failed").0
}

async fn send(
    ws: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    msg: Value,
) {
    ws.send(Message::Text(msg.to_string().into()))
        .await
        .expect("WS send failed");
}

/// Collect all non-buffer_state messages within `ms` milliseconds.
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
                if let Ok(v) = serde_json::from_str::<Value>(&t) {
                    if v["type"] != "buffer_state" {
                        out.push(v);
                    }
                }
            }
            _ => break,
        }
    }
    out
}

/// Collect only buffer_state messages within `ms` milliseconds.
async fn recv_buffer_states(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    ms: u64,
) -> Vec<Value> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
    #[allow(clippy::while_let_loop)]
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(v) = serde_json::from_str::<Value>(&t) {
                    if v["type"] == "buffer_state" {
                        out.push(v);
                    }
                }
            }
            _ => break,
        }
    }
    out
}

/// Wait for the first message matching `type_filter`, ignoring others.
async fn recv_first_of(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    type_filter: &str,
    ms: u64,
) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(v) = serde_json::from_str::<Value>(&t) {
                    if v["type"] == type_filter {
                        return Some(v);
                    }
                }
            }
            _ => return None,
        }
    }
}

// ── Synthetic stream fixture ──────────────────────────────────────────────────

/// Sends enough delta text to produce `n` complete sentences and flush them.
/// Each sentence has a period so the chunk splitter fires immediately.
async fn stream_n_sentences(
    ws: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    n: usize,
) {
    for i in 0..n {
        let sentence = format!("Sentence {}. ", i + 1);
        send(ws, json!({"type": "delta", "text": sentence})).await;
    }
}

// ── Pipeline-level tests (dry_run) ───────────────────────────────────────────
//
// In dry_run mode synthesis is synchronous and emits `speak` immediately.
// These tests verify the stream_state + pipeline are reset on signal.

#[tokio::test]
async fn reset_stops_pipeline_no_more_speak_events() {
    let url = start_server().await;
    let mut ws = connect_dry(&url).await;

    // Start a turn and get at least one speak.
    send(
        &mut ws,
        json!({"type": "delta", "text": "First sentence. "}),
    )
    .await;
    let first = recv_first_of(&mut ws, "speak", 1500).await;
    assert!(
        first.is_some(),
        "expected a speak event for the first sentence"
    );

    // Buffer PARTIAL text (no sentence boundary — stays in buffer, not yet drained).
    send(
        &mut ws,
        json!({"type": "delta", "text": "This text should be silenced"}),
    )
    .await;

    // Reset before message_end: clears the stream buffer.
    send(&mut ws, json!({"type": "reset"})).await;

    // Flush the now-empty state — nothing to speak.
    send(&mut ws, json!({"type": "message_end"})).await;

    let after = recv_all(&mut ws, 800).await;
    let speaks: Vec<_> = after.iter().filter(|m| m["type"] == "speak").collect();
    assert!(
        speaks.is_empty(),
        "buffered partial text must not produce speak after reset, got: {speaks:?}"
    );
}

#[tokio::test]
async fn mute_stops_pipeline_no_more_speak_events() {
    let url = start_server().await;
    let mut ws = connect_dry(&url).await;

    // Start a turn.
    send(
        &mut ws,
        json!({"type": "delta", "text": "First sentence. "}),
    )
    .await;
    let first = recv_first_of(&mut ws, "speak", 1500).await;
    assert!(first.is_some(), "expected first speak");

    // Buffer PARTIAL text (stays in buffer, not yet drained).
    send(
        &mut ws,
        json!({"type": "delta", "text": "This should be silenced"}),
    )
    .await;

    // Mute before message_end: server is now disabled.
    send(&mut ws, json!({"type": "set_config", "enabled": false})).await;

    // Flush: server is disabled so it clears the buffer without synthesising.
    send(&mut ws, json!({"type": "message_end"})).await;

    let after = recv_all(&mut ws, 800).await;
    let speaks: Vec<_> = after.iter().filter(|m| m["type"] == "speak").collect();
    assert!(
        speaks.is_empty(),
        "buffered partial text must not produce speak after mute, got: {speaks:?}"
    );
}

#[tokio::test]
async fn reset_then_new_turn_produces_fresh_output() {
    let url = start_server().await;
    let mut ws = connect_dry(&url).await;

    // First turn.
    send(&mut ws, json!({"type": "delta", "text": "First turn. "})).await;
    let _ = recv_first_of(&mut ws, "speak", 1500).await;

    // Reset.
    send(&mut ws, json!({"type": "reset"})).await;

    // New turn must work — not blocked or confused.
    send(&mut ws, json!({"type": "delta", "text": "Fresh turn. "})).await;
    let fresh = recv_first_of(&mut ws, "speak", 1500).await;
    assert!(fresh.is_some(), "new turn after reset must produce speak");
    let text = fresh.unwrap()["text"].as_str().unwrap_or("").to_owned();
    assert!(
        text.contains("Fresh"),
        "speak text should contain fresh turn content, got: {text}"
    );
}

#[tokio::test]
async fn reset_cancels_multiple_buffered_fragments() {
    // Verify that multiple partial fragments accumulated across several delta
    // messages are all cancelled by a single reset before message_end flushes them.
    let url = start_server().await;
    let mut ws = connect_dry(&url).await;

    // Confirm the pipeline is alive with one complete sentence.
    send(&mut ws, json!({"type": "delta", "text": "Active. "})).await;
    let first = recv_first_of(&mut ws, "speak", 1500).await;
    assert!(first.is_some(), "pipeline must be alive");

    // Accumulate fragments WITHOUT sentence boundaries — all stay buffered.
    for i in 0..5_u32 {
        let frag = format!("Fragment {i} ");
        send(&mut ws, json!({"type": "delta", "text": frag})).await;
    }

    // Reset wipes the stream buffer.
    send(&mut ws, json!({"type": "reset"})).await;

    // Flush the now-empty state.
    send(&mut ws, json!({"type": "message_end"})).await;

    let after = recv_all(&mut ws, 800).await;
    let speaks: Vec<_> = after.iter().filter(|m| m["type"] == "speak").collect();
    assert!(
        speaks.is_empty(),
        "all buffered fragments must be cancelled by reset, got: {speaks:?}"
    );
}

// ── Queue-level tests (MockSynthBackend) ──────────────────────────────────────
//
// These tests use the real WS streaming path with instant synthesis (MockSynthBackend).
// `playing` events are emitted when chunks are enqueued.
// `play_wav_async` will fail in CI (no audio device) — the player logs a warning
// and continues; this does not affect queue observability.

#[tokio::test]
async fn mock_synth_turn_produces_playing_events() {
    let url = start_server().await;
    let mut ws = connect_synth(&url).await;

    send(&mut ws, json!({"type": "delta", "text": "Hello world. "})).await;
    let msg = recv_first_of(&mut ws, "playing", 5000).await;
    assert!(msg.is_some(), "mock synth must emit playing event");
}

#[tokio::test]
async fn reset_drains_play_queue_no_playing_events_after() {
    // NOTE: This test documents the DESIRED behaviour after the fix.
    // Currently FAILS because PlayQueue::stop() is a no-op — the 32-item
    // mpsc channel continues draining after reset.
    let url = start_server().await;
    let mut ws = connect_synth(&url).await;

    // Fill the pipeline with many sentences.
    stream_n_sentences(&mut ws, 8).await;

    // Wait for the first playing event (first chunk enqueued).
    let first = recv_first_of(&mut ws, "playing", 5000).await;
    assert!(first.is_some(), "should get at least one playing event");

    // Send reset immediately.
    send(&mut ws, json!({"type": "reset"})).await;

    // After reset, no further playing events should arrive for the old turn.
    // Allow a brief window (one in-flight chunk is acceptable).
    let after = recv_all(&mut ws, 400).await;
    let playing: Vec<_> = after.iter().filter(|m| m["type"] == "playing").collect();
    assert!(
        playing.len() <= 1,
        "at most 1 in-flight playing allowed after reset, got {}: {playing:?}",
        playing.len()
    );
}

#[tokio::test]
async fn mute_drains_play_queue_no_playing_events_after() {
    // NOTE: Documents desired behaviour after fix.
    // Currently FAILS for same reason as reset_drains_play_queue.
    let url = start_server().await;
    let mut ws = connect_synth(&url).await;

    stream_n_sentences(&mut ws, 8).await;

    let first = recv_first_of(&mut ws, "playing", 5000).await;
    assert!(first.is_some(), "should get at least one playing event");

    // Mute.
    send(&mut ws, json!({"type": "set_config", "enabled": false})).await;

    let after = recv_all(&mut ws, 400).await;
    let playing: Vec<_> = after.iter().filter(|m| m["type"] == "playing").collect();
    assert!(
        playing.len() <= 1,
        "at most 1 in-flight playing allowed after mute, got {}: {playing:?}",
        playing.len()
    );
}

#[tokio::test]
async fn reset_emits_empty_buffer_state() {
    // NOTE: Documents desired behaviour after fix.
    // Currently FAILS because the reset handler does not reset PlaybackBuffer
    // or emit a buffer_state event.
    let url = start_server().await;
    let mut ws = connect_synth(&url).await;

    stream_n_sentences(&mut ws, 4).await;

    // Wait for at least one playing event so some chunks are enqueued.
    let _ = recv_first_of(&mut ws, "playing", 5000).await;

    // Reset.
    send(&mut ws, json!({"type": "reset"})).await;

    // Collect buffer_state events in the next 400ms.
    // After reset, a buffer_state with complete=false and slots=[] must arrive.
    let states = recv_buffer_states(&mut ws, 400).await;
    let cleared = states.iter().any(|s| {
        let slots = s["data"]["slots"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(false);
        let pending = s["data"]["pending"].as_u64().unwrap_or(1) == 0;
        let buffered = s["data"]["buffered"].as_u64().unwrap_or(1) == 0;
        slots && pending && buffered
    });
    assert!(
        cleared,
        "a buffer_state with empty slots, 0 pending, 0 buffered must arrive after reset. got: {states:?}"
    );
}

#[tokio::test]
async fn reset_then_new_mock_turn_is_not_blocked() {
    // After reset, the server must accept and process a new turn without delay.
    let url = start_server().await;
    let mut ws = connect_synth(&url).await;

    // First turn — 3 sentences.
    stream_n_sentences(&mut ws, 3).await;
    let _ = recv_first_of(&mut ws, "playing", 5000).await;

    // Reset.
    send(&mut ws, json!({"type": "reset"})).await;

    // Second turn must start quickly — not blocked by the old play queue.
    let t0 = tokio::time::Instant::now();
    send(&mut ws, json!({"type": "delta", "text": "New turn. "})).await;
    let second = recv_first_of(&mut ws, "playing", 3000).await;
    let elapsed = t0.elapsed();

    assert!(
        second.is_some(),
        "new turn after reset must produce playing"
    );
    assert!(
        elapsed.as_secs() < 2,
        "new turn should start quickly after reset, took {elapsed:?}"
    );
}
