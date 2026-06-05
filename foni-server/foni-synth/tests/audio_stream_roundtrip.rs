//! Round-trip: known audio clips → concat with silence gaps → audio_stream chunker → verify splits.
//!
//! Proves that silence boundaries in audio produce the same chunk splits
//! as punctuation does in text streaming.

use foni_synth::engine::audio_stream;

const SR: u32 = 16_000;

fn sine_tone(duration_ms: usize, freq: f32, amplitude: f32) -> Vec<f32> {
    let n = SR as usize * duration_ms / 1000;
    (0..n)
        .map(|i| amplitude * (2.0 * std::f32::consts::PI * freq * i as f32 / SR as f32).sin())
        .collect()
}

fn silence(duration_ms: usize) -> Vec<f32> {
    vec![0.0f32; SR as usize * duration_ms / 1000]
}

#[test]
fn three_utterances_three_chunks() {
    let utterances = [
        sine_tone(2000, 200.0, 0.5),
        sine_tone(1500, 300.0, 0.4),
        sine_tone(3000, 250.0, 0.6),
    ];

    let mut full = Vec::new();
    for u in &utterances {
        full.extend_from_slice(u);
        full.extend_from_slice(&silence(600));
    }

    let mut state = audio_stream::fresh_state();
    let result = audio_stream::feed_audio(&mut state, &full);
    let mut chunks = result.chunks;
    if let Some(last) = audio_stream::flush(&mut state) {
        chunks.push(last);
    }

    eprintln!(
        "Input: {} utterances, {:.1}s total",
        utterances.len(),
        full.len() as f32 / SR as f32
    );
    for (i, c) in chunks.iter().enumerate() {
        eprintln!("  chunk {}: {:.2}s", i + 1, c.len() as f32 / SR as f32);
    }

    assert_eq!(chunks.len(), utterances.len(), "one chunk per utterance");
}

#[test]
fn short_noise_between_speech_is_not_a_boundary() {
    let mut full = Vec::new();
    full.extend_from_slice(&sine_tone(2000, 200.0, 0.5));
    full.extend_from_slice(&silence(100)); // 100ms — too short to split
    full.extend_from_slice(&sine_tone(2000, 300.0, 0.5));
    full.extend_from_slice(&silence(600)); // real boundary

    let mut state = audio_stream::fresh_state();
    let result = audio_stream::feed_audio(&mut state, &full);
    let mut chunks = result.chunks;
    if let Some(last) = audio_stream::flush(&mut state) {
        chunks.push(last);
    }

    assert_eq!(chunks.len(), 1, "100ms gap should not split");
    let dur = chunks[0].len() as f32 / SR as f32;
    assert!(dur > 3.5, "single chunk should be ~4s, got {dur:.1}s");
}

#[test]
fn varying_gap_lengths() {
    // 200ms gap = no split, 600ms gap = split, 800ms gap = split
    let mut full = Vec::new();
    full.extend_from_slice(&sine_tone(1000, 200.0, 0.5));
    full.extend_from_slice(&silence(200)); // no split
    full.extend_from_slice(&sine_tone(1000, 250.0, 0.5));
    full.extend_from_slice(&silence(600)); // split
    full.extend_from_slice(&sine_tone(1000, 300.0, 0.5));
    full.extend_from_slice(&silence(800)); // split

    let mut state = audio_stream::fresh_state();
    let result = audio_stream::feed_audio(&mut state, &full);
    let mut chunks = result.chunks;
    if let Some(last) = audio_stream::flush(&mut state) {
        chunks.push(last);
    }

    eprintln!("Chunks: {}", chunks.len());
    for (i, c) in chunks.iter().enumerate() {
        eprintln!("  chunk {}: {:.2}s", i + 1, c.len() as f32 / SR as f32);
    }

    assert_eq!(chunks.len(), 2, "200ms gap merges, 400ms and 800ms split");
}

#[test]
fn real_wav_clips_split_correctly() {
    let clip_paths = [
        "../../dataset/diomedes/001.wav",
        "../../dataset/diomedes/006.wav",
        "../../dataset/diomedes/009.wav",
    ];

    // Check if test data exists
    if !std::path::Path::new(clip_paths[0]).exists() {
        eprintln!("  skipped (no dataset/diomedes/)");
        return;
    }

    // Decode and find actual sample rate
    let first_bytes = std::fs::read(clip_paths[0]).unwrap();
    let first_wav = foni_analyse::decode_wav(&first_bytes).unwrap();
    let clip_sr = first_wav.sample_rate;

    let mut full = Vec::new();
    let gap = vec![0.0f32; (clip_sr as f32 * 0.6) as usize];

    for path in &clip_paths {
        let bytes = std::fs::read(path).unwrap();
        let wav = foni_analyse::decode_wav(&bytes).unwrap();
        full.extend_from_slice(&wav.samples);
        full.extend_from_slice(&gap);
    }

    let total = full.len() as f32 / clip_sr as f32;
    eprintln!(
        "Real clips: {:.1}s from {} files ({}Hz)",
        total,
        clip_paths.len(),
        clip_sr
    );

    let mut state = audio_stream::fresh_state_with_rate(clip_sr);
    let result = audio_stream::feed_audio(&mut state, &full);
    let mut chunks = result.chunks;
    if let Some(last) = audio_stream::flush(&mut state) {
        chunks.push(last);
    }

    eprintln!("Chunks: {}", chunks.len());
    for (i, c) in chunks.iter().enumerate() {
        eprintln!("  chunk {}: {:.2}s", i + 1, c.len() as f32 / SR as f32);
    }

    assert_eq!(
        chunks.len(),
        clip_paths.len(),
        "expected one chunk per WAV clip"
    );
}
