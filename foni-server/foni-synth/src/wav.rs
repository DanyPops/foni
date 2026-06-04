/// WAV encode/decode utilities for foni-synth.
/// Decode is thin wrapper over foni-analyse; encode uses hound.
use foni_analyse::wav::WavData;
use hound::{SampleFormat, WavSpec, WavWriter};

pub use foni_analyse::decode_wav;

/// Encode f32 mono samples to a 16-bit PCM WAV buffer.
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, hound::Error> {
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut buf = Vec::new();
    let mut writer = WavWriter::new(std::io::Cursor::new(&mut buf), spec)?;
    for &s in samples {
        let s16 = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(s16)?;
    }
    writer.finalize()?;
    Ok(buf)
}

/// Prepend and append `pad_secs` of silence to mono f32 samples.
pub fn pad_silence(samples: &[f32], pad_secs: f32, sample_rate: u32) -> Vec<f32> {
    if pad_secs <= 0.0 {
        return samples.to_vec();
    }
    let pad_n = (sample_rate as f32 * pad_secs) as usize;
    let mut out = vec![0f32; pad_n];
    out.extend_from_slice(samples);
    out.extend(vec![0f32; pad_n]);
    out
}

/// Round-trip: WAV bytes → f32 samples → DSP → WAV bytes.
/// Convenience used by /process and /breath routes.
pub fn roundtrip<F>(wav_bytes: &[u8], f: F) -> Result<Vec<u8>, String>
where
    F: FnOnce(&mut Vec<f32>, u32),
{
    let WavData {
        mut samples,
        sample_rate,
        ..
    } = decode_wav(wav_bytes).map_err(|e| e.to_string())?;
    f(&mut samples, sample_rate);
    encode_wav(&samples, sample_rate).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_wav_produces_valid_wav() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 / 100.0).sin()).collect();
        let wav = encode_wav(&samples, 16000).expect("encode");
        assert!(wav.len() > 44);
        assert_eq!(&wav[..4], b"RIFF");
    }

    #[test]
    fn encode_empty_samples() {
        let wav = encode_wav(&[], 16000).expect("encode");
        assert_eq!(&wav[..4], b"RIFF");
    }

    #[test]
    fn encode_clamps_out_of_range() {
        let samples = vec![-2.0, 0.0, 2.0];
        let wav = encode_wav(&samples, 16000).expect("encode");
        assert!(wav.len() > 44);
    }

    #[test]
    fn pad_silence_adds_padding() {
        let samples = vec![1.0; 100];
        let padded = pad_silence(&samples, 0.5, 16000);
        let pad_n = (16000.0 * 0.5) as usize;
        assert_eq!(padded.len(), 100 + 2 * pad_n);
        assert_eq!(padded[0], 0.0);
        assert_eq!(padded[pad_n], 1.0);
        assert_eq!(*padded.last().unwrap(), 0.0);
    }

    #[test]
    fn pad_silence_zero_is_noop() {
        let samples = vec![1.0; 100];
        let padded = pad_silence(&samples, 0.0, 16000);
        assert_eq!(padded.len(), 100);
    }

    #[test]
    fn pad_silence_negative_is_noop() {
        let samples = vec![1.0; 100];
        let padded = pad_silence(&samples, -1.0, 16000);
        assert_eq!(padded.len(), 100);
    }

    #[test]
    fn roundtrip_applies_transform() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 / 100.0).sin() * 0.5).collect();
        let wav = encode_wav(&samples, 16000).expect("encode");
        let result = roundtrip(&wav, |s, _sr| {
            for x in s.iter_mut() {
                *x *= 2.0;
            }
        });
        assert!(result.is_ok());
    }

    #[test]
    fn roundtrip_invalid_wav_returns_error() {
        let result = roundtrip(b"not a wav", |_, _| {});
        assert!(result.is_err());
    }
}

const TAIL_TRIM_THRESHOLD_DB: f32 = -30.0;
const TAIL_TRIM_FRAME_MS: usize = 100;
const TAIL_TRIM_MIN_SILENT_FRAMES: usize = 2;

/// Trim trailing silence/breath from audio samples.
/// Walks backwards from the end, finds where energy drops below threshold
/// for N consecutive frames, and cuts there.
pub fn trim_tail(samples: &mut Vec<f32>, sample_rate: u32) {
    let frame_size = (sample_rate as usize * TAIL_TRIM_FRAME_MS) / 1000;
    if samples.len() < frame_size * 2 {
        return;
    }

    let threshold = 10.0_f32.powf(TAIL_TRIM_THRESHOLD_DB / 20.0);
    let n_frames = samples.len() / frame_size;
    let mut last_loud_frame = n_frames;

    for i in (0..n_frames).rev() {
        let start = i * frame_size;
        let end = (start + frame_size).min(samples.len());
        let rms =
            (samples[start..end].iter().map(|s| s * s).sum::<f32>() / (end - start) as f32).sqrt();

        if rms >= threshold {
            last_loud_frame = i + 1;
            break;
        }
    }

    let silent_tail_frames = n_frames - last_loud_frame;
    if silent_tail_frames >= TAIL_TRIM_MIN_SILENT_FRAMES {
        let trim_to = (last_loud_frame + 1) * frame_size;
        samples.truncate(trim_to.min(samples.len()));
    }
}

#[cfg(test)]
mod tail_tests {
    use super::*;

    #[test]
    fn trim_removes_silent_tail() {
        let sr = 24000;
        let mut samples: Vec<f32> = (0..sr * 2)
            .map(|i| {
                if i < sr {
                    (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.5
                } else {
                    0.001 * (i as f32 / sr as f32) // quiet tail
                }
            })
            .collect();
        let original_len = samples.len();
        trim_tail(&mut samples, sr as u32);
        assert!(samples.len() < original_len, "should have trimmed");
    }

    #[test]
    fn trim_keeps_all_speech() {
        let sr = 24000;
        let mut samples: Vec<f32> = (0..sr)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.5)
            .collect();
        let original_len = samples.len();
        trim_tail(&mut samples, sr as u32);
        assert_eq!(samples.len(), original_len, "no silent tail to trim");
    }

    #[test]
    fn trim_handles_short_audio() {
        let mut samples = vec![0.1, 0.2, 0.3];
        trim_tail(&mut samples, 24000);
        assert_eq!(samples.len(), 3, "too short to trim");
    }
}
