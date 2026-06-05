//! Audio stream chunker — the audio equivalent of `stream.rs`.
//!
//! Text stream:  tokens → buffer → boundary (. ! ?) → emit sentence
//! Audio stream: frames → buffer → boundary (silence) → emit utterance
//!
//! Both turn a continuous stream into discrete chunks for batch processing.

const FRAME_SIZE: usize = 480; // 30ms at 16kHz
const SAMPLE_RATE: u32 = 16_000;

/// Silence threshold in linear RMS amplitude (~-30dB).
const SILENCE_THRESHOLD: f32 = 0.03;

/// Minimum silence duration to consider a boundary (in frames).
const MIN_SILENCE_FRAMES: usize = 10; // 300ms

/// Minimum speech duration to emit (in frames).
const MIN_SPEECH_FRAMES: usize = 17; // ~500ms

/// Maximum utterance before forced flush (in frames).
const MAX_UTTERANCE_FRAMES: usize = 500; // ~15s

#[derive(Debug, Clone)]
pub struct AudioStreamState {
    buffer: Vec<f32>,
    silence_count: usize,
    speech_count: usize,
    is_speaking: bool,
}

pub struct DrainResult {
    /// Complete utterances ready for processing.
    pub chunks: Vec<Vec<f32>>,
    /// Partial utterance still accumulating.
    pub buffered_secs: f32,
}

impl Default for AudioStreamState {
    fn default() -> Self {
        Self {
            buffer: Vec::with_capacity(SAMPLE_RATE as usize * 5),
            silence_count: 0,
            speech_count: 0,
            is_speaking: false,
        }
    }
}

pub fn fresh_state() -> AudioStreamState {
    AudioStreamState::default()
}

/// Feed raw audio samples (mono, 16kHz). Returns chunks when silence boundaries detected.
pub fn feed_audio(state: &mut AudioStreamState, samples: &[f32]) -> DrainResult {
    let mut chunks = Vec::new();

    for frame_start in (0..samples.len()).step_by(FRAME_SIZE) {
        let frame_end = (frame_start + FRAME_SIZE).min(samples.len());
        let frame = &samples[frame_start..frame_end];

        let rms = frame_rms(frame);
        let is_speech = rms > SILENCE_THRESHOLD;

        if is_speech {
            if !state.is_speaking {
                state.is_speaking = true;
                state.silence_count = 0;
            }
            state.speech_count += 1;
            state.silence_count = 0;
            state.buffer.extend_from_slice(frame);
        } else if state.is_speaking {
            state.silence_count += 1;
            state.buffer.extend_from_slice(frame);

            if state.silence_count >= MIN_SILENCE_FRAMES {
                if state.speech_count >= MIN_SPEECH_FRAMES {
                    let trim_samples = state.silence_count * FRAME_SIZE;
                    let keep = state.buffer.len().saturating_sub(trim_samples);
                    let utterance = state.buffer[..keep].to_vec();
                    chunks.push(utterance);
                }
                state.buffer.clear();
                state.speech_count = 0;
                state.silence_count = 0;
                state.is_speaking = false;
            }
        }

        if state.speech_count >= MAX_UTTERANCE_FRAMES {
            let utterance = std::mem::take(&mut state.buffer);
            chunks.push(utterance);
            state.speech_count = 0;
            state.silence_count = 0;
            state.is_speaking = false;
        }
    }

    DrainResult {
        chunks,
        buffered_secs: state.buffer.len() as f32 / SAMPLE_RATE as f32,
    }
}

/// Force-flush any buffered audio (end of stream).
pub fn flush(state: &mut AudioStreamState) -> Option<Vec<f32>> {
    if state.speech_count >= MIN_SPEECH_FRAMES {
        let utterance = std::mem::take(&mut state.buffer);
        state.speech_count = 0;
        state.silence_count = 0;
        state.is_speaking = false;
        Some(utterance)
    } else {
        state.buffer.clear();
        state.speech_count = 0;
        state.silence_count = 0;
        state.is_speaking = false;
        None
    }
}

fn frame_rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silence(duration_ms: usize) -> Vec<f32> {
        vec![0.0f32; SAMPLE_RATE as usize * duration_ms / 1000]
    }

    fn tone(duration_ms: usize, amplitude: f32) -> Vec<f32> {
        let n = SAMPLE_RATE as usize * duration_ms / 1000;
        (0..n)
            .map(|i| {
                amplitude
                    * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin()
            })
            .collect()
    }

    #[test]
    fn silence_produces_no_chunks() {
        let mut state = fresh_state();
        let result = feed_audio(&mut state, &silence(1000));
        assert!(result.chunks.is_empty());
    }

    #[test]
    fn short_speech_discarded() {
        let mut state = fresh_state();
        // 200ms speech + 500ms silence → too short, discarded
        let mut audio = tone(200, 0.5);
        audio.extend(silence(500));
        let result = feed_audio(&mut state, &audio);
        assert!(result.chunks.is_empty());
    }

    #[test]
    fn speech_then_silence_emits_chunk() {
        let mut state = fresh_state();
        // 1s speech + 500ms silence → should emit
        let mut audio = tone(1000, 0.5);
        audio.extend(silence(500));
        let result = feed_audio(&mut state, &audio);
        assert_eq!(result.chunks.len(), 1);
        // Chunk should be roughly 1s of audio (silence trimmed)
        let chunk_secs = result.chunks[0].len() as f32 / SAMPLE_RATE as f32;
        assert!(
            chunk_secs > 0.8 && chunk_secs < 1.2,
            "chunk was {chunk_secs}s"
        );
    }

    #[test]
    fn two_utterances_two_chunks() {
        let mut state = fresh_state();
        let mut audio = tone(1000, 0.5);
        audio.extend(silence(500));
        audio.extend(tone(800, 0.5));
        audio.extend(silence(500));
        let result = feed_audio(&mut state, &audio);
        assert_eq!(result.chunks.len(), 2);
    }

    #[test]
    fn flush_emits_buffered_speech() {
        let mut state = fresh_state();
        feed_audio(&mut state, &tone(1000, 0.5));
        // No silence yet, nothing emitted
        assert!(state.buffer.len() > 0);
        let flushed = flush(&mut state);
        assert!(flushed.is_some());
    }

    #[test]
    fn flush_discards_short_buffer() {
        let mut state = fresh_state();
        feed_audio(&mut state, &tone(200, 0.5));
        let flushed = flush(&mut state);
        assert!(flushed.is_none());
    }

    #[test]
    fn max_utterance_forces_flush() {
        let mut state = fresh_state();
        // 16s continuous speech → should force-flush at ~15s
        let result = feed_audio(&mut state, &tone(16000, 0.5));
        assert!(!result.chunks.is_empty());
    }

    #[test]
    fn buffered_secs_tracks_partial() {
        let mut state = fresh_state();
        let result = feed_audio(&mut state, &tone(2000, 0.5));
        assert!(result.chunks.is_empty());
        assert!(result.buffered_secs > 1.5);
    }
}
