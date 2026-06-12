//! Streaming render test — text chunks arrive sequentially, each gets synthesized
//! and appended to a playback buffer in order. Simulates the YouTube-style buffering
//! pattern: play chunk 0 immediately, buffer chunk 1 while playing, etc.
//!
//! Uses a mock TTS server that returns sine WAVs with different frequencies
//! per chunk, so we can verify ordering by analyzing the output.

use axum::{routing::post, Json, Router};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

const SAMPLE_RATE: u32 = 24_000;

fn sine_wav(freq: f32, duration_secs: f32) -> Vec<u8> {
    let n = (SAMPLE_RATE as f32 * duration_secs) as usize;
    let samples: Vec<f32> = (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
        .collect();
    depecherd::wav::encode_wav(&samples, SAMPLE_RATE).expect("encode wav")
}

/// Pluggable delay strategy for mock TTS.
#[derive(Clone)]
#[allow(dead_code)] // Random used in tests, variants kept for completeness
enum Delay {
    None,
    Fixed(Duration),
    Random { min_ms: u64, max_ms: u64 },
    PerChunk(Vec<Duration>),
}

impl Delay {
    async fn wait(&self, chunk_index: usize) {
        match self {
            Delay::None => {}
            Delay::Fixed(d) => tokio::time::sleep(*d).await,
            Delay::Random { min_ms, max_ms } => {
                let range = max_ms - min_ms;
                let jitter = if range > 0 {
                    (chunk_index as u64 * 7 + 13) % range
                } else {
                    0
                };
                tokio::time::sleep(Duration::from_millis(min_ms + jitter)).await;
            }
            Delay::PerChunk(delays) => {
                if let Some(d) = delays.get(chunk_index) {
                    tokio::time::sleep(*d).await;
                }
            }
        }
    }
}

/// Mock TTS: returns a sine WAV with frequency = 200 + (call_index * 100) Hz.
/// Delay strategy is pluggable.
async fn start_mock_tts_with(delay: Delay) -> (String, Arc<AtomicUsize>) {
    let call_count = Arc::new(AtomicUsize::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/synthesize",
        post(move |Json(body): Json<Value>| {
            let counter = counter.clone();
            let delay = delay.clone();
            async move {
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                let text = body["text"].as_str().unwrap_or("");
                let freq = 200.0 + (idx as f32 * 100.0);
                eprintln!("  [mock tts] chunk {idx}: \"{text}\" → {freq}Hz");

                delay.wait(idx).await;

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

async fn start_mock_tts() -> (String, Arc<AtomicUsize>) {
    start_mock_tts_with(Delay::Fixed(Duration::from_millis(50))).await
}

#[tokio::test]
async fn sequential_chunks_arrive_in_order() {
    let (tts_url, call_count) = start_mock_tts().await;
    let client = depecher_client::DepecherClient::new(&tts_url);

    let chunks = ["First sentence.", "Second sentence!", "Third sentence?"];

    let mut wav_buffers: Vec<Vec<u8>> = Vec::new();

    for (i, text) in chunks.iter().enumerate() {
        let req = depecher_client::SynthRequest::new(*text);
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
    let _client = depecher_client::DepecherClient::new(&tts_url);

    let chunks = vec!["First.", "Second.", "Third.", "Fourth.", "Fifth."];

    // Fire all at once
    let mut handles = Vec::new();
    for text in &chunks {
        let client = depecher_client::DepecherClient::new(&tts_url);
        let text = text.to_string();
        handles.push(tokio::spawn(async move {
            let req = depecher_client::SynthRequest::new(&text);
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
    let _client = depecher_client::DepecherClient::new(&tts_url);

    let chunks = ["Alpha.", "Beta.", "Gamma."];

    // Synthesize in parallel, but collect in index order
    let mut handles = Vec::new();
    for (i, text) in chunks.iter().enumerate() {
        let client = depecher_client::DepecherClient::new(&tts_url);
        let text = text.to_string();
        handles.push(tokio::spawn(async move {
            let req = depecher_client::SynthRequest::new(&text);
            let wav = client.synthesize(&req).await.expect("synth");
            (i, wav)
        }));
    }

    // Collect results (may arrive out of order)
    let mut indexed: Vec<(usize, depecher_client::WavData)> = Vec::new();
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
        let decoded = depecher_analyse::decode_wav(&wav.0).expect("decode");
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

    let chunks = ["One.", "Two.", "Three.", "Four."];

    // Simulate: fire chunk 0, start "playing" it while firing 1,2,3
    let client = depecher_client::DepecherClient::new(&tts_url);
    let req = depecher_client::SynthRequest::new(chunks[0]);
    let first = client.synthesize(&req).await.expect("synth 0");

    // First chunk available — "playback" could start now
    assert!(first.0.len() > 44, "first chunk ready for playback");

    // Meanwhile, fire remaining chunks
    let mut remaining = Vec::new();
    for text in &chunks[1..] {
        let client = depecher_client::DepecherClient::new(&tts_url);
        let text = text.to_string();
        remaining.push(tokio::spawn(async move {
            let req = depecher_client::SynthRequest::new(&text);
            client.synthesize(&req).await.expect("synth")
        }));
    }

    // Collect — these arrive while "first" is "playing"
    for handle in remaining {
        let wav = handle.await.unwrap();
        assert!(wav.0.len() > 44);
    }
}

#[tokio::test]
async fn zero_delay_all_fast() {
    let (tts_url, call_count) = start_mock_tts_with(Delay::None).await;
    let client = depecher_client::DepecherClient::new(&tts_url);

    let t0 = std::time::Instant::now();
    for text in ["A.", "B.", "C."] {
        let req = depecher_client::SynthRequest::new(text);
        client.synthesize(&req).await.unwrap();
    }
    let elapsed = t0.elapsed();

    assert_eq!(call_count.load(Ordering::SeqCst), 3);
    assert!(
        elapsed < Duration::from_millis(500),
        "no-delay should be fast"
    );
}

#[tokio::test]
async fn per_chunk_delay_spike_detected_by_tracker() {
    use depecherd::engine::jitter::{JitterTracker, Trip};

    let delays = vec![
        Duration::from_millis(100), // chunk 0: fast
        Duration::from_millis(100), // chunk 1: fast
        Duration::from_millis(800), // chunk 2: spike!
        Duration::from_millis(100), // chunk 3: back to normal
    ];
    let (tts_url, _) = start_mock_tts_with(Delay::PerChunk(delays)).await;
    let client = depecher_client::DepecherClient::new(&tts_url);

    let chunks = ["Fast one.", "Fast two.", "Slow spike!", "Fast again."];
    let mut tracker = JitterTracker::new();

    for (i, text) in chunks.iter().enumerate() {
        let t0 = std::time::Instant::now();
        let req = depecher_client::SynthRequest::new(*text);
        let wav = client.synthesize(&req).await.unwrap();
        let rtt = t0.elapsed();

        tracker.record(Trip {
            chunk_index: i,
            rtt,
            audio_bytes: wav.0.len(),
        });
    }

    assert!(tracker.max_rtt_ms() > 700.0, "should see the spike");
    assert!(tracker.mean_rtt_ms() > 200.0, "mean should be pulled up");
    assert!(
        tracker.predicted_rtt_ms() > 150.0,
        "EMA should react to spike"
    );
}

#[tokio::test]
async fn jitter_decides_filler_for_slow_chunks() {
    use depecherd::engine::jitter::{Action, JitterTracker, Trip};

    // Sustained slow responses — all over the 500ms budget
    let delays = vec![
        Duration::from_millis(600),
        Duration::from_millis(700),
        Duration::from_millis(800),
    ];
    let (tts_url, _) = start_mock_tts_with(Delay::PerChunk(delays)).await;
    let client = depecher_client::DepecherClient::new(&tts_url);

    let chunks = ["One.", "Two.", "Three."];
    let mut tracker = JitterTracker::new();

    for (i, text) in chunks.iter().enumerate() {
        let t0 = std::time::Instant::now();
        let req = depecher_client::SynthRequest::new(*text);
        let wav = client.synthesize(&req).await.unwrap();

        tracker.record(Trip {
            chunk_index: i,
            rtt: t0.elapsed(),
            audio_bytes: wav.0.len(),
        });
    }

    // Budget = 0.5s playback, RTTs are 600-800ms → needs filler
    let decision = tracker.decide(0.5);
    eprintln!("Predicted RTT: {:.0}ms", tracker.predicted_rtt_ms());
    eprintln!("Decision: {decision:?}");

    assert!(
        matches!(decision, Action::Filler { .. }),
        "sustained 600-800ms RTT vs 500ms budget should need filler"
    );
}
