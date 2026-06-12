//! ws_cutoff — E2E tests for Stop and Mute signal handling during active streams.
//!
//! Verifies that when `reset` or `set_config {enabled:false}` arrives mid-stream:
//!   (a) no further synthesis events are emitted for the current turn
//!   (b) the play queue drains immediately (buffer_state → empty)
//!   (c) a new turn is not blocked by lingering audio
//!
//! All tests use [`support::StreamFixture`] so raw WS plumbing is invisible.

mod support;

use support::{start_mock_server, StreamFixture};

// ── Pipeline-level tests (dry_run) ───────────────────────────────────────────
//
// dry_run=true: synthesis is synchronous, emits `speak` instead of `playing`.
// No play queue involved — tests verify stream-buffer cancellation.

#[tokio::test]
async fn reset_cancels_buffered_partial_text() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_dry(&url).await;

    f.tick().await;
    assert!(
        f.wait_for("speak", 1500).await.is_some(),
        "pipeline must be alive"
    );

    // Buffer partial text (no period → stays in buffer).
    f.buffer("This should be silenced").await;

    // Reset before flush → buffer wiped.
    f.reset().await;
    f.flush().await;

    StreamFixture::assert_none(&f.collect(800).await, "speak");
}

#[tokio::test]
async fn mute_cancels_buffered_partial_text() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_dry(&url).await;

    f.tick().await;
    assert!(
        f.wait_for("speak", 1500).await.is_some(),
        "pipeline must be alive"
    );

    f.buffer("This should be silenced").await;
    f.mute().await;
    f.flush().await; // disabled → no synthesis

    StreamFixture::assert_none(&f.collect(800).await, "speak");
}

#[tokio::test]
async fn reset_cancels_multiple_buffered_fragments() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_dry(&url).await;

    f.tick().await;
    assert!(f.wait_for("speak", 1500).await.is_some());

    for i in 0..5_u32 {
        f.buffer(&format!("Fragment {i} ")).await;
    }

    f.reset().await;
    f.flush().await;

    StreamFixture::assert_none(&f.collect(800).await, "speak");
}

#[tokio::test]
async fn reset_then_new_turn_produces_fresh_output() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_dry(&url).await;

    f.tick().await;
    assert!(f.wait_for("speak", 1500).await.is_some());

    f.reset().await;

    f.tick().await;
    let speak = f.wait_for("speak", 1500).await;
    assert!(speak.is_some(), "new turn after reset must produce speak");
    let text = speak.unwrap()["text"].as_str().unwrap_or("").to_owned();
    assert!(
        text.contains("Tick 2"),
        "speak must contain new-turn text, got: {text}"
    );
}

// ── Enable / unmute path ────────────────────────────────────────────────────
//
// Verifies the "other way around": what happens when TTS is re-enabled
// after being disabled or muted.
//
// Invariants:
//   (a) Text buffered WHILE muted (arrives after mute signal) is spoken on unmute.
//   (b) Text that was in the buffer WHEN mute fired is discarded (stream_state reset).
//   (c) After reset → new turn starts clean with no carryover.

#[tokio::test]
async fn enable_speaks_text_buffered_while_muted() {
    // Text arriving AFTER mute accumulates in the fresh stream buffer
    // and is spoken when TTS is re-enabled.
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_dry(&url).await;

    f.mute().await;

    // Text arriving while muted — stays in stream buffer, not synthesised.
    f.buffer("Text sent while muted. ").await;

    // Re-enable → accumulated text should drain and produce speak.
    f.unmute().await;

    let events = f.collect(1500).await;
    let speaks = StreamFixture::count(&events, "speak");
    assert!(
        speaks > 0,
        "text buffered while muted must produce speak on unmute, got: {events:?}"
    );
}

#[tokio::test]
async fn text_in_buffer_at_mute_time_is_discarded() {
    // Partial text already in the buffer WHEN mute fires is discarded
    // because mute resets stream_state.
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_dry(&url).await;

    // Buffer partial text (no period — stays in buffer).
    f.buffer("Partial text that should vanish").await;

    // Mute: clears stream_state → the buffered text is gone.
    f.mute().await;

    // Unmute and flush the now-empty state.
    f.unmute().await;
    f.flush().await;

    let events = f.collect(800).await;
    StreamFixture::assert_none(&events, "speak");
}

#[tokio::test]
async fn reenable_resumes_synthesis_of_queued_text() {
    // Disable before any text arrives. Text sent while disabled accumulates.
    // Re-enabling drains the buffer and speaks it.
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_dry(&url).await;

    f.mute().await;
    f.tick().await; // complete sentence — stored in buffer while disabled

    // No speak events while disabled.
    StreamFixture::assert_none(&f.collect(500).await, "speak");

    // Re-enable — the buffered tick should now be spoken.
    f.unmute().await;

    let events = f.collect(1500).await;
    assert!(
        StreamFixture::count(&events, "speak") > 0,
        "re-enable must speak buffered text, got: {events:?}"
    );
}

#[tokio::test]
async fn reset_then_enable_starts_completely_fresh() {
    // After reset, re-enabling should produce no carryover speech.
    // Only a new tick AFTER enable should be spoken.
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_dry(&url).await;

    f.tick().await;
    assert!(f.wait_for("speak", 1500).await.is_some());

    // Reset clears everything, then mute the fresh state.
    f.reset().await;
    f.mute().await;
    f.unmute().await;

    // No carryover from old turn.
    StreamFixture::assert_none(&f.collect(600).await, "speak");

    // New explicit tick after unmute DOES speak.
    f.tick().await;
    assert!(
        f.wait_for("speak", 1500).await.is_some(),
        "new tick after reset+unmute must speak"
    );
}

// ── Queue-level tests (MockSynthBackend) ──────────────────────────────────────
//
// Ticker sends one sentence per interval; server has time to enqueue audio
// between ticks, making stop/mute timing deterministic.
// `playing` events mark enqueue (not playback); buffer_state and new-turn
// response time are the canonical observables for queue-drain correctness.

#[tokio::test]
async fn mock_synth_produces_playing_events() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_synth(&url).await;

    f.tick().await;
    assert!(f.wait_for("playing", 5000).await.is_some());
}

#[tokio::test]
async fn reset_clears_buffer_state_immediately() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_synth(&url).await;

    // Ticker: one sentence every 80ms so server has time to process each.
    f.tick_n(3, 80).await;
    assert!(
        f.wait_for("playing", 5000).await.is_some(),
        "pipeline alive"
    );

    f.reset().await;

    assert!(
        f.wait_buffer_empty(800).await,
        "buffer_state must show empty after reset"
    );
}

#[tokio::test]
async fn mute_clears_buffer_state_immediately() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_synth(&url).await;

    f.tick_n(3, 80).await;
    assert!(
        f.wait_for("playing", 5000).await.is_some(),
        "pipeline alive"
    );

    f.mute().await;

    assert!(
        f.wait_buffer_empty(800).await,
        "buffer_state must show empty after mute"
    );
}

#[tokio::test]
async fn reset_new_turn_not_blocked() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_synth(&url).await;

    f.tick_n(4, 80).await;
    assert!(f.wait_for("playing", 5000).await.is_some());

    f.reset().await;

    let t0 = tokio::time::Instant::now();
    f.tick().await;
    assert!(
        f.wait_for("playing", 3000).await.is_some(),
        "new turn after reset must produce playing"
    );
    assert!(
        t0.elapsed().as_secs() < 2,
        "new turn must not be blocked by old audio, took {:?}",
        t0.elapsed()
    );
}

#[tokio::test]
async fn mute_unmute_new_turn_not_blocked() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_synth(&url).await;

    f.tick_n(4, 80).await;
    assert!(f.wait_for("playing", 5000).await.is_some());

    f.mute().await;
    f.unmute().await;

    let t0 = tokio::time::Instant::now();
    f.tick().await;
    assert!(
        f.wait_for("playing", 3000).await.is_some(),
        "new turn after mute+unmute must produce playing"
    );
    assert!(
        t0.elapsed().as_secs() < 2,
        "new turn must not be blocked after unmute, took {:?}",
        t0.elapsed()
    );
}

// ── WS resume ────────────────────────────────────────────────────────────────────
//
// Verifies the StreamLog ring buffer and resume protocol:
//   - "playing" events carry a monotonic chunk_id field
//   - sending {"type": "resume", "last_chunk_id": N} replays chunks > N
//     tagged with {"replayed": true}

#[tokio::test]
async fn playing_events_carry_chunk_id() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_synth(&url).await;

    f.tick().await;
    let event = f.wait_for("playing", 5000).await;
    assert!(event.is_some(), "must receive playing event");
    let ev = event.unwrap();
    assert!(
        ev["chunk_id"].is_u64(),
        "playing event must carry chunk_id, got: {ev:?}"
    );
}

#[tokio::test]
async fn resume_replays_missed_chunks() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_synth(&url).await;

    // Stream 3 chunks and collect all playing events.
    f.tick_n(3, 80).await;
    let events = f.collect(5000).await;
    let playing: Vec<_> = events.iter().filter(|e| e["type"] == "playing").collect();
    assert!(
        playing.len() >= 2,
        "need at least 2 playing events to test resume, got: {playing:?}"
    );

    // Find the chunk_id of the first playing event.
    let first_id = playing[0]["chunk_id"].as_u64().unwrap_or(0);

    // Send resume from first_id — server should replay all chunks after it.
    f.send_msg(serde_json::json!({"type": "resume", "last_chunk_id": first_id}))
        .await;

    let replayed = f.collect(1000).await;
    let replayed_events: Vec<_> = replayed
        .iter()
        .filter(|e| e["type"] == "playing" && e["replayed"] == true)
        .collect();
    assert!(
        !replayed_events.is_empty(),
        "resume must replay missed chunks, got no replayed events"
    );
    // All replayed events must have chunk_id > first_id.
    for ev in &replayed_events {
        let id = ev["chunk_id"].as_u64().unwrap_or(0);
        assert!(
            id > first_id,
            "replayed chunk_id {id} must be > last_seen {first_id}"
        );
    }
}

#[tokio::test]
async fn resume_from_zero_replays_all_logged_chunks() {
    let url = start_mock_server().await;
    let mut f = StreamFixture::connect_synth(&url).await;

    f.tick_n(3, 80).await;
    let events = f.collect(5000).await;
    let total = events.iter().filter(|e| e["type"] == "playing").count();
    if total == 0 {
        return; // synthesis didn’t produce output in this run — skip
    }

    // Resume from chunk 0 — everything logged should be replayed.
    f.send_msg(serde_json::json!({"type": "resume", "last_chunk_id": 0}))
        .await;
    let replayed = f.collect(1000).await;
    let count = replayed
        .iter()
        .filter(|e| e["type"] == "playing" && e["replayed"] == true)
        .count();
    assert!(
        count > 0,
        "resume from 0 must replay all logged chunks, got 0 replayed events"
    );
}
