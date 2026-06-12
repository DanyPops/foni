//! Audio stream chunker — the audio equivalent of `stream.rs`.
//!
//! Text stream:  tokens → buffer → boundary (. ! ?) → emit sentence
//! Audio stream: frames → buffer → boundary (silence) → emit utterance
//!
//! Both turn a continuous stream into discrete chunks for batch processing.

/// Silence threshold in linear RMS amplitude (~-30dB).
const SILENCE_THRESHOLD: f32 = 0.03;

/// Timing constants (in seconds, converted to frames at runtime).
const FRAME_SECS: f32 = 0.03; // 30ms per frame
const MIN_SILENCE_SECS: f32 = 0.4; // gap to split on
const MIN_SPEECH_SECS: f32 = 0.5; // discard shorter
const MAX_UTTERANCE_SECS: f32 = 15.0; // force-flush

#[derive(Debug, Clone)]
pub struct AudioStreamState {
    buffer: Vec<f32>,
    silence_count: usize,
    speech_count: usize,
    is_speaking: bool,
    frame_size: usize,
    min_silence_frames: usize,
    min_speech_frames: usize,
    max_utterance_frames: usize,
}

pub struct DrainResult {
    /// Complete utterances ready for processing.
    pub chunks: Vec<Vec<f32>>,
    /// Partial utterance still accumulating.
    pub buffered_secs: f32,
}

pub fn fresh_state() -> AudioStreamState {
    fresh_state_with_rate(16_000)
}

pub fn fresh_state_with_rate(sample_rate: u32) -> AudioStreamState {
    let frame_size = (sample_rate as f32 * FRAME_SECS) as usize;
    AudioStreamState {
        buffer: Vec::with_capacity(sample_rate as usize * 5),
        silence_count: 0,
        speech_count: 0,
        is_speaking: false,
        frame_size,
        min_silence_frames: (MIN_SILENCE_SECS / FRAME_SECS) as usize,
        min_speech_frames: (MIN_SPEECH_SECS / FRAME_SECS) as usize,
        max_utterance_frames: (MAX_UTTERANCE_SECS / FRAME_SECS) as usize,
    }
}

/// Feed raw audio samples (mono, 16kHz). Returns chunks when silence boundaries detected.
pub fn feed_audio(state: &mut AudioStreamState, samples: &[f32]) -> DrainResult {
    let mut chunks = Vec::new();

    let frame_size = state.frame_size;
    for frame_start in (0..samples.len()).step_by(frame_size) {
        let frame_end = (frame_start + frame_size).min(samples.len());
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

            if state.silence_count >= state.min_silence_frames {
                if state.speech_count >= state.min_speech_frames {
                    let trim_samples = state.silence_count * frame_size;
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

        if state.speech_count >= state.max_utterance_frames {
            let utterance = std::mem::take(&mut state.buffer);
            chunks.push(utterance);
            state.speech_count = 0;
            state.silence_count = 0;
            state.is_speaking = false;
        }
    }

    DrainResult {
        chunks,
        buffered_secs: state.buffer.len() as f32 / (state.frame_size as f32 / FRAME_SECS),
    }
}

/// Force-flush any buffered audio (end of stream).
pub fn flush(state: &mut AudioStreamState) -> Option<Vec<f32>> {
    if state.speech_count >= state.min_speech_frames {
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

    const SR: u32 = 16_000;

    fn silence(duration_ms: usize) -> Vec<f32> {
        vec![0.0f32; SR as usize * duration_ms / 1000]
    }

    fn tone(duration_ms: usize, amplitude: f32) -> Vec<f32> {
        let n = SR as usize * duration_ms / 1000;
        (0..n)
            .map(|i| amplitude * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SR as f32).sin())
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
        let chunk_secs = result.chunks[0].len() as f32 / SR as f32;
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
        assert!(!state.buffer.is_empty());
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
