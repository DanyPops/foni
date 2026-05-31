use hound::WavReader;
use std::io::Cursor;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WavError {
    #[error("hound decode: {0}")]
    Hound(#[from] hound::Error),
    #[error("unsupported bit depth {0} — expected 16 or 32")]
    UnsupportedBitDepth(u16),
}

pub struct WavData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Decode a WAV buffer into normalised f32 mono samples.
/// Stereo is downmixed to mono by averaging channels.
pub fn decode_wav(bytes: &[u8]) -> Result<WavData, WavError> {
    let mut reader = WavReader::new(Cursor::new(bytes))?;
    let spec = reader.spec();

    let interleaved: Vec<f32> = match spec.bits_per_sample {
        16 => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<Result<_, _>>()?,
        32 => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / i32::MAX as f32))
            .collect::<Result<_, _>>()?,
        b => return Err(WavError::UnsupportedBitDepth(b)),
    };

    let ch = spec.channels as usize;
    let samples = if ch == 1 {
        interleaved
    } else {
        interleaved
            .chunks_exact(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    };

    Ok(WavData {
        samples,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buf = Vec::new();
        let mut w = hound::WavWriter::new(std::io::Cursor::new(&mut buf), spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
        buf
    }

    #[test]
    fn round_trip_silence() {
        let wav = make_wav(&vec![0i16; 2205], 22050);
        let d = decode_wav(&wav).unwrap();
        assert_eq!(d.sample_rate, 22050);
        assert!(d.samples.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn normalises_max_amplitude() {
        let wav = make_wav(&[i16::MAX], 22050);
        let d = decode_wav(&wav).unwrap();
        assert!((d.samples[0] - 1.0).abs() < 1e-4);
    }
}
