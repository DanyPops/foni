//! Failing regression tests for the three mute/stop bugs.
//! Each test must remain red until its corresponding fix lands.

mod support;

use std::time::Duration;

use serde_json::json;
use support::{start_slow_mock_server, StreamFixture};

#[tokio::test]
async fn bug1_reset_during_in_flight_synthesis_is_ignored() {
    let url = start_slow_mock_server(500).await;
    let mut f = StreamFixture::connect_synth(&url).await;

    f.send_msg(json!({"type": "delta", "text": "Hello world. "}))
        .await;
    f.reset().await;

    StreamFixture::assert_none(&f.collect(900).await, "playing");
}

#[tokio::test]
async fn bug1_mute_during_in_flight_synthesis_is_ignored() {
    let url = start_slow_mock_server(500).await;
    let mut f = StreamFixture::connect_synth(&url).await;

    f.send_msg(json!({"type": "delta", "text": "Hello world. "}))
        .await;
    f.mute().await;

    StreamFixture::assert_none(&f.collect(900).await, "playing");
}

#[tokio::test]
async fn bug2_stale_chunk_tagged_with_new_generation_plays_anyway() {
    use foni_synth::engine::facade::PlayQueue;

    let (queue, _handle) = PlayQueue::new();

    // generation_snapshot() does not exist yet → compile error
    let snap_gen = queue.generation_snapshot();

    queue.clear();

    // enqueue() loads the current (post-clear) generation, so the chunk
    // passes the player's guard and plays despite the reset.
    queue.enqueue(vec![0u8; 44]).await;

    tokio::time::sleep(Duration::from_millis(150)).await;

    assert_eq!(
        snap_gen, 0,
        "snapshot must capture the pre-clear generation"
    );
}

#[tokio::test]
async fn bug3_clear_cannot_stop_currently_playing_subprocess() {
    // play_wav_spawn does not exist yet → compile error.
    // Replacing play_wav_blocking with a spawn-based variant gives a Child
    // handle that clear() can kill.
    let tmp = std::env::temp_dir().join("bug3_test.wav");
    let _child = foni_synth::engine::player::play_wav_spawn(&tmp);
}

#[tokio::test]
async fn bug3_new_turn_after_reset_is_not_delayed_by_playing_audio() {
    let url = start_slow_mock_server(0).await;
    let mut f = StreamFixture::connect_synth(&url).await;

    f.tick().await;
    assert!(
        f.wait_for("playing", 3000).await.is_some(),
        "pipeline alive"
    );

    // Let the player task dequeue and enter play_wav_async before resetting.
    tokio::time::sleep(Duration::from_millis(80)).await;

    f.reset().await;

    let t0 = tokio::time::Instant::now();
    f.tick().await;
    assert!(
        f.wait_for("playing", 500).await.is_some(),
        "new turn after reset must not be delayed by the previous audio process"
    );
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "player task blocked by unkilled subprocess: {:?}",
        t0.elapsed()
    );
}
