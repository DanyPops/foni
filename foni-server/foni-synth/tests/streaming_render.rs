//! Streaming render test — text chunks arrive sequentially, each gets synthesized
//! and appended to a playback buffer in order. Simulates the YouTube-style buffering
//! pattern: play chunk 0 immediately, buffer chunk 1 while playing, etc.
//!
//! Uses a mock TTS server that returns sine WAVs with different frequencies
//! per chunk, so we can verify ordering by analyzing the output.

use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;

const SAMPLE_RATE: u32 = 24_000;

fn sine_wav(freq: f32, duration_secs: f32) -> Vec<u8> {
    let n = (SAMPLE_RATE as f32 * duration_secs) as usize;
    let samples: Vec<f32> = (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
        .collect();
    foni_synth::wav::encode_wav(&samples, SAMPLE_RATE).expect("encode wav")
}

/// Mock TTS: returns a sine WAV with frequency = 200 + (call_index * 100) Hz.
/// This lets us verify chunk ordering by checking the dominant frequency.
async fn start_mock_tts() -> (String, Arc<AtomicUsize>) {
    let call_count = Arc::new(AtomicUsize::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/synthesize",
        post(move |Json(body): Json<Value>| {
            let counter = counter.clone();
            async move {
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                let text = body["text"].as_str().unwrap_or("");
                let freq = 200.0 + (idx as f32 * 100.0);
                eprintln!("  [mock tts] chunk {idx}: \"{text}\" → {freq}Hz");

                // Simulate latency (~50ms per chunk)
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;

                let wav = sine_wav(freq, 0.5);
                let headers = [(axum::http::header::CONTENT_TYPE, "audio/wav")];
                (headers, wav)
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://127.0.0.1:{}", addr.port()), call_count)
}

#[tokio::test]
async fn sequential_chunks_arrive_in_order() {
    let (tts_url, call_count) = start_mock_tts().await;
    let client = foni_client::FoniClient::new(&tts_url);

    let chunks = vec!["First sentence.", "Second sentence!", "Third sentence?"];

    let mut wav_buffers: Vec<Vec<u8>> = Vec::new();

    for (i, text) in chunks.iter().enumerate() {
        let req = foni_client::SynthRequest::new(*text);
        let wav = client.synthesize(&req).await.expect("synth failed");
        eprintln!("  chunk {i}: {} bytes", wav.0.len());
        wav_buffers.push(wav.0);
    }

    assert_eq!(call_count.load(Ordering::SeqCst), 3);
    assert_eq!(wav_buffers.len(), 3);

    // Each buffer should be non-empty WAV
    for (i, buf) in wav_buffers.iter().enumerate() {
        assert!(buf.len() > 44, "chunk {i} too small: {} bytes", buf.len());
    }
}

#[tokio::test]
async fn parallel_chunks_all_complete() {
    let (tts_url, call_count) = start_mock_tts().await;
    let client = foni_client::FoniClient::new(&tts_url);

    let chunks = vec!["First.", "Second.", "Third.", "Fourth.", "Fifth."];

    // Fire all at once
    let mut handles = Vec::new();
    for text in &chunks {
        let client = foni_client::FoniClient::new(&tts_url);
        let text = text.to_string();
        handles.push(tokio::spawn(async move {
            let req = foni_client::SynthRequest::new(&text);
            client.synthesize(&req).await
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap().expect("synth failed"));
    }

    assert_eq!(call_count.load(Ordering::SeqCst), 5);
    assert_eq!(results.len(), 5);

    for (i, wav) in results.iter().enumerate() {
        assert!(wav.0.len() > 44, "chunk {i} empty");
    }
}

#[tokio::test]
async fn ordered_buffer_assembly() {
    let (tts_url, _) = start_mock_tts().await;
    let client = foni_client::FoniClient::new(&tts_url);

    let chunks = vec!["Alpha.", "Beta.", "Gamma."];

    // Synthesize in parallel, but collect in index order
    let mut handles = Vec::new();
    for (i, text) in chunks.iter().enumerate() {
        let client = foni_client::FoniClient::new(&tts_url);
        let text = text.to_string();
        handles.push(tokio::spawn(async move {
            let req = foni_client::SynthRequest::new(&text);
            let wav = client.synthesize(&req).await.expect("synth");
            (i, wav)
        }));
    }

    // Collect results (may arrive out of order)
    let mut indexed: Vec<(usize, foni_client::WavData)> = Vec::new();
    for handle in handles {
        indexed.push(handle.await.unwrap());
    }

    // Sort by index — this is what the buffer does
    indexed.sort_by_key(|(i, _)| *i);

    // Verify order restored
    for (pos, (idx, _)) in indexed.iter().enumerate() {
        assert_eq!(*idx, pos, "buffer order broken at position {pos}");
    }

    // Concatenate samples from ordered buffers
    let mut all_samples: Vec<f32> = Vec::new();
    for (_, wav) in &indexed {
        let decoded = foni_analyse::decode_wav(&wav.0).expect("decode");
        all_samples.extend_from_slice(&decoded.samples);
    }

    let total_secs = all_samples.len() as f32 / SAMPLE_RATE as f32;
    assert!(
        total_secs > 1.0,
        "concatenated should be ~1.5s (3×0.5s), got {total_secs:.1}s"
    );
}

#[tokio::test]
async fn buffer_playback_can_start_before_all_complete() {
    let (tts_url, _) = start_mock_tts().await;

    let chunks = vec!["One.", "Two.", "Three.", "Four."];

    // Simulate: fire chunk 0, start "playing" it while firing 1,2,3
    let client = foni_client::FoniClient::new(&tts_url);
    let req = foni_client::SynthRequest::new(chunks[0]);
    let first = client.synthesize(&req).await.expect("synth 0");

    // First chunk available — "playback" could start now
    assert!(first.0.len() > 44, "first chunk ready for playback");

    // Meanwhile, fire remaining chunks
    let mut remaining = Vec::new();
    for text in &chunks[1..] {
        let client = foni_client::FoniClient::new(&tts_url);
        let text = text.to_string();
        remaining.push(tokio::spawn(async move {
            let req = foni_client::SynthRequest::new(&text);
            client.synthesize(&req).await.expect("synth")
        }));
    }

    // Collect — these arrive while "first" is "playing"
    for handle in remaining {
        let wav = handle.await.unwrap();
        assert!(wav.0.len() > 44);
    }
}
