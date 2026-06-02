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
