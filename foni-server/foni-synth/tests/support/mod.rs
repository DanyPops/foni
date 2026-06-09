//! Shared test infrastructure for WS integration tests.
//!
//! [`StreamFixture`] wraps a WebSocket connection to foni-synth and provides
//! a high-level API for pipeline and queue-drain scenarios:
//!
//! * per-connection dry_run / lang configuration (no env-var coupling)
//! * [`StreamFixture::tick`] / [`StreamFixture::tick_n`] — controlled sentence feed
//! * [`StreamFixture::reset`] / [`StreamFixture::mute`] / [`StreamFixture::unmute`]
//! * [`StreamFixture::collect`] / [`StreamFixture::wait_for`] — event collection
//! * [`StreamFixture::wait_buffer_empty`] — asserts the PlaybackBuffer was cleared
//!
//! # Server construction
//!
//! Use the module-level helpers:
//! ```ignore
//! let url = start_mock_server().await;            // MockSynthBackend, real WS path
//! let mut f = StreamFixture::connect_dry(&url).await;  // dry_run pipeline tests
//! let mut f = StreamFixture::connect_synth(&url).await; // real-synth queue tests
//! ```

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

// ── Server factory ────────────────────────────────────────────────────────────

/// Spawn a foni-synth server with [`MockSynthBackend`] on a random port.
///
/// No env-var side effects — `FONI_DRY_RUN` is explicitly cleared so
/// per-connection `set_config {dry_run}` controls the mode.
pub async fn start_mock_server() -> String {
    start_slow_mock_server(0).await
}

/// Spawn a foni-synth server whose [`MockSynthBackend`] artificially delays
/// every synthesis call by `delay_ms` milliseconds.
///
/// Used to widen the window between synthesis-start and synthesis-complete
/// so control messages (reset/mute) can be sent while synthesis is in flight.
pub async fn start_slow_mock_server(delay_ms: u64) -> String {
    use std::sync::Arc;
    std::env::remove_var("FONI_DRY_RUN");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let synth: foni_synth::engine::synth_backend::SharedSynth = if delay_ms == 0 {
        foni_synth::engine::synth_backend::mock_backend()
    } else {
        Arc::new(foni_synth::engine::synth_backend::MockSynthBackend::with_delay(delay_ms))
    };
    let app = foni_synth::build_router_with_synth(synth).await;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("ws://127.0.0.1:{}/ws", addr.port())
}

// ── StreamFixture ─────────────────────────────────────────────────────────────

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Controlled WS session against a foni-synth server.
pub struct StreamFixture {
    ws: Ws,
    tick_count: u32,
}

impl StreamFixture {
    // ── Constructors ──────────────────────────────────────────────────────────

    /// Connect in dry-run mode — `speak` events instead of real synthesis.
    /// Skips translation so no Ollama calls.
    pub async fn connect_dry(url: &str) -> Self {
        let mut f = Self::raw_connect(url).await;
        f.send_msg(json!({"type": "set_config", "dry_run": true}))
            .await;
        f
    }

    /// Connect in real-synth mode — MockSynthBackend produces instant WAVs,
    /// `playing` events are emitted, `play_wav_async` fails silently (no device).
    /// Translation disabled via same-lang config.
    pub async fn connect_synth(url: &str) -> Self {
        let mut f = Self::raw_connect(url).await;
        // ru,ru → skip Ollama translation so tests don't hit the network.
        f.send_msg(json!({"type": "set_config", "lang": "ru,ru"}))
            .await;
        f
    }

    async fn raw_connect(url: &str) -> Self {
        let (ws, _) = connect_async(url).await.expect("WS connect failed");
        Self { ws, tick_count: 0 }
    }

    // ── Ticker ────────────────────────────────────────────────────────────────

    /// Send one complete sentence (period → drains immediately from stream buffer).
    pub async fn tick(&mut self) {
        self.tick_count += 1;
        let n = self.tick_count;
        self.send_msg(json!({ "type": "delta", "text": format!("Tick {n}. ") }))
            .await;
    }

    /// Send `n` ticks spaced `interval_ms` apart using a tokio interval timer.
    ///
    /// The interval gives the server time to process each sentence before the next
    /// arrives, making stop/mute assertions deterministic.
    pub async fn tick_n(&mut self, n: usize, interval_ms: u64) {
        let mut clock = tokio::time::interval(Duration::from_millis(interval_ms));
        for _ in 0..n {
            clock.tick().await;
            self.tick().await;
        }
    }

    /// Buffer partial text (no sentence boundary → stays in buffer until flush or reset).
    pub async fn buffer(&mut self, text: &str) {
        self.send_msg(json!({ "type": "delta", "text": text }))
            .await;
    }

    /// Flush the stream buffer (message_end).
    pub async fn flush(&mut self) {
        self.send_msg(json!({ "type": "message_end" })).await;
    }

    // ── Control signals ───────────────────────────────────────────────────────

    pub async fn reset(&mut self) {
        self.send_msg(json!({ "type": "reset" })).await;
    }

    pub async fn mute(&mut self) {
        self.send_msg(json!({ "type": "set_config", "enabled": false }))
            .await;
    }

    pub async fn unmute(&mut self) {
        self.send_msg(json!({ "type": "set_config", "enabled": true }))
            .await;
    }

    // ── Event collection ──────────────────────────────────────────────────────

    /// Collect all non-buffer_state messages arriving within `ms` milliseconds.
    pub async fn collect(&mut self, ms: u64) -> Vec<Value> {
        self.collect_filtered(ms, |v| v["type"] != "buffer_state")
            .await
    }

    /// Collect only `buffer_state` messages arriving within `ms` milliseconds.
    pub async fn collect_buffer_states(&mut self, ms: u64) -> Vec<Value> {
        self.collect_filtered(ms, |v| v["type"] == "buffer_state")
            .await
    }

    /// Wait for the first message of the given type, up to `timeout_ms`.
    pub async fn wait_for(&mut self, event_type: &str, timeout_ms: u64) -> Option<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            match tokio::time::timeout_at(deadline, self.ws.next()).await {
                Ok(Some(Ok(Message::Text(t)))) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&t) {
                        if v["type"] == event_type {
                            return Some(v);
                        }
                    }
                }
                _ => return None,
            }
        }
    }

    /// Wait until a `buffer_state` with 0 slots, 0 pending, 0 buffered arrives.
    ///
    /// Returns `true` if the empty state arrived within `timeout_ms`.
    /// This is the canonical observable for "play queue was cleared".
    pub async fn wait_buffer_empty(&mut self, timeout_ms: u64) -> bool {
        let states = self.collect_buffer_states(timeout_ms).await;
        states.iter().any(|s| {
            s["data"]["slots"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(false)
                && s["data"]["pending"].as_u64().unwrap_or(1) == 0
                && s["data"]["buffered"].as_u64().unwrap_or(1) == 0
        })
    }

    // ── Assertion helpers ─────────────────────────────────────────────────────

    /// Count events of `type_` in a slice.
    pub fn count(events: &[Value], type_: &str) -> usize {
        events.iter().filter(|e| e["type"] == type_).count()
    }

    /// Assert no events of `type_` are present. Panics with context on failure.
    pub fn assert_none(events: &[Value], type_: &str) {
        let found: Vec<_> = events.iter().filter(|e| e["type"] == type_).collect();
        assert!(
            found.is_empty(),
            "expected no {type_} events, got {}: {found:?}",
            found.len()
        );
    }

    // ── Raw send ──────────────────────────────────────────────────────────────

    pub async fn send_msg(&mut self, msg: Value) {
        self.ws
            .send(Message::Text(msg.to_string().into()))
            .await
            .expect("WS send failed");
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    async fn collect_filtered<F>(&mut self, ms: u64, pred: F) -> Vec<Value>
    where
        F: Fn(&Value) -> bool,
    {
        let mut out = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
        #[allow(clippy::while_let_loop)]
        loop {
            match tokio::time::timeout_at(deadline, self.ws.next()).await {
                Ok(Some(Ok(Message::Text(t)))) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&t) {
                        if pred(&v) {
                            out.push(v);
                        }
                    }
                }
                _ => break,
            }
        }
        out
    }
}
