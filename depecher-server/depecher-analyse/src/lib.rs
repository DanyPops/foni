pub mod alignment;
pub mod contour;
pub mod gap;
pub mod loudness;
pub mod mcd;
pub mod mfcc;
pub mod naturalness;
pub mod pitch;
pub mod report;
pub mod speaker_sim;
pub mod spectral;
pub mod spectral_timeline;
pub mod tempo;
pub mod temporal;
pub mod timeline;
pub mod tone;
pub mod voice;
pub mod voice_id;
pub mod wav;
pub mod wer;

pub use alignment::{
    align, format_alignment_table, AlignedPair, TimelineComparison, TimelineFixture,
};
pub use contour::compute_contour_correlations;
pub use gap::{compute_gap, GapResult, GapRow, TargetTensor, Verdict};
pub use loudness::energy_envelope;
pub use loudness::LoudnessMetrics;
pub use mcd::compute_mcd;
pub use mfcc::mfcc_distance;
pub use pitch::compute_with_contour;
pub use pitch::fast_f0_stats;
pub use pitch::PitchMetrics;

pub use report::{format_gap_summary, format_gap_table};
pub use speaker_sim::{
    cosine_similarity, embed as speaker_embed, speaker_similarity, SpeakerEmbedding,
};
pub use spectral::SpectralMetrics;
pub use temporal::TemporalMetrics;
pub use timeline::{pauses, segment, voiced_segments, Segment, SegmentKind, Timeline};
pub use voice::VoiceMetrics;
pub use wav::decode_wav;
pub use wer::{compute_wer, edit_distance_words, transcribe, WerResult};

use serde::Serialize;

/// Full analysis result for one audio buffer.
/// Serialised to JSON by the /analyse HTTP endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisResult {
    pub temporal: TemporalMetrics,
    pub spectral: SpectralMetrics,
    pub loudness: LoudnessMetrics,
    pub pitch: PitchMetrics,
    pub voice: VoiceMetrics,
    /// F0 per 10ms frame in Hz. 0.0 = unvoiced. Used for contour correlation.
    pub f0_contour: Vec<f32>,
    /// RMS per 10ms frame (linear amplitude). Used for energy envelope correlation.
    pub energy_envelope: Vec<f32>,
}

/// How close does the synthesis sound to the studio reference?
/// All scores: higher = better match.
#[derive(Debug, Clone, Serialize)]
pub struct ComparisonResult {
    /// Per-metric gap table — shows which acoustic dimensions are off and by how much.
    pub gap: GapResult,
    /// Timbre distance in dB — how different the voice texture sounds. < 6 = good, < 4 = excellent.
    pub timbre_distance_db: f32,
    /// How closely the pitch shape (rise/fall pattern) matches the reference. 1.0 = identical.
    pub pitch_shape_match: f32,
    /// How closely the loudness envelope matches. 1.0 = identical.
    pub loudness_shape_match: f32,
    /// Word Error Rate (%) — how much of the text was understood correctly.
    pub wer_pct: Option<f32>,
    /// Does it sound like the same person? 0–1, from voice texture (MFCC-based).
    pub voice_match: Option<f32>,
    /// How natural does it sound? 1–5 scale (like a human listener rating).
    /// Computed by Google ViSQOL comparing synthesis against studio recording.
    pub naturalness: Option<f32>,
    /// Does it sound like Sidorovich? 0–1, from neural voice fingerprint.
    /// Requires the voice ID model — run `just setup-voice-id` once.
    pub sounds_like: Option<f32>,
    /// Per-frame spectral comparison — shows WHERE the quality gap lives.
    /// Only computed when `compare_full` is called with samples available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline: Option<spectral_timeline::SpectralTimeline>,
}

/// Compare a synthesis against a reference recording.
/// Both must be analysed with `analyse()` first.
/// `syn_wav_bytes` is the raw WAV for Whisper WER (pass &[] to skip WER).
pub fn compare(
    phrase: &str,
    synthesis: &AnalysisResult,
    reference: &AnalysisResult,
    ref_samples: &[f32],
    syn_samples: &[f32],
    sample_rate: u32,
    syn_wav_bytes: &[u8],
) -> ComparisonResult {
    compare_with_paths(
        phrase,
        synthesis,
        reference,
        ref_samples,
        syn_samples,
        sample_rate,
        syn_wav_bytes,
        None,
        None,
    )
}

/// Like `compare` but also runs ViSQOL when file paths are available.
#[allow(clippy::too_many_arguments)]
pub fn compare_with_paths(
    phrase: &str,
    synthesis: &AnalysisResult,
    reference: &AnalysisResult,
    ref_samples: &[f32],
    syn_samples: &[f32],
    sample_rate: u32,
    syn_wav_bytes: &[u8],
    ref_path: Option<&str>,
    syn_path: Option<&str>,
) -> ComparisonResult {
    compare_full(
        phrase,
        synthesis,
        reference,
        ref_samples,
        syn_samples,
        sample_rate,
        syn_wav_bytes,
        ref_path,
        syn_path,
        None,
    )
}

/// Full comparison including optional ECAPA session for speaker similarity.
#[allow(clippy::too_many_arguments)]
pub fn compare_full(
    phrase: &str,
    synthesis: &AnalysisResult,
    reference: &AnalysisResult,
    ref_samples: &[f32],
    syn_samples: &[f32],
    sample_rate: u32,
    syn_wav_bytes: &[u8],
    ref_path: Option<&str>,
    syn_path: Option<&str>,
    ecapa_session: Option<&mut ort::session::Session>,
) -> ComparisonResult {
    let tensor = gap::TargetTensor::from_analysis(reference, phrase);
    let gap = gap::compute_gap(phrase, synthesis, &tensor);
    let timbre_distance_db = mcd::compute_mcd(ref_samples, syn_samples, sample_rate);
    let (pitch_shape_match, loudness_shape_match) = contour::compute_contour_correlations(
        &reference.f0_contour,
        &reference.energy_envelope,
        &synthesis.f0_contour,
        &synthesis.energy_envelope,
    );
    let wer_pct = if syn_wav_bytes.is_empty() {
        None
    } else {
        wer::compute_wer(syn_wav_bytes, phrase, "ru").map(|r| r.wer_pct)
    };
    let voice_match = {
        let ref_embed = speaker_sim::embed(ref_samples, sample_rate, "reference");
        let syn_embed = speaker_sim::embed(syn_samples, sample_rate, "synthesis");
        Some(speaker_sim::speaker_similarity(&ref_embed, &syn_embed))
    };
    let naturalness = match (ref_path, syn_path) {
        (Some(r), Some(s)) => naturalness::score(r, s),
        _ => None,
    };
    let sounds_like = ecapa_session.and_then(|sess: &mut ort::session::Session| {
        let ref_16k = voice_id::to_16k(ref_samples, sample_rate);
        let syn_16k = voice_id::to_16k(syn_samples, sample_rate);
        let ref_emb = voice_id::extract(sess, &ref_16k)?;
        let syn_emb = voice_id::extract(sess, &syn_16k)?;
        Some(voice_id::cosine_sim(&ref_emb, &syn_emb))
    });

    let timeline = Some(spectral_timeline::compare(
        ref_samples,
        syn_samples,
        sample_rate,
        &reference.f0_contour,
        &synthesis.f0_contour,
        &reference.energy_envelope,
        &synthesis.energy_envelope,
    ));

    ComparisonResult {
        gap,
        timbre_distance_db,
        pitch_shape_match,
        loudness_shape_match,
        wer_pct,
        voice_match,
        naturalness,
        sounds_like,
        timeline,
    }
}

/// Run the full analysis pipeline on raw f32 samples.
pub fn analyse(samples: &[f32], sample_rate: u32) -> AnalysisResult {
    let t0 = std::time::Instant::now();

    let t_loudness = std::time::Instant::now();
    let loudness = loudness::compute(samples, sample_rate);
    let energy_envelope = loudness::energy_envelope(samples, sample_rate);
    tracing::debug!(stage = "loudness", ms = t_loudness.elapsed().as_millis());

    let t_spectral = std::time::Instant::now();
    let spectral = spectral::compute(samples, sample_rate);
    tracing::debug!(stage = "spectral", ms = t_spectral.elapsed().as_millis());

    let t_temporal = std::time::Instant::now();
    let temporal = temporal::compute(samples, sample_rate);
    tracing::debug!(stage = "temporal", ms = t_temporal.elapsed().as_millis());

    let t_voice = std::time::Instant::now();
    let voice = voice::compute(samples, sample_rate);
    tracing::debug!(stage = "voice", ms = t_voice.elapsed().as_millis());

    let t_pitch = std::time::Instant::now();
    let (pitch, f0_contour) = pitch::compute_with_contour(samples, sample_rate);
    tracing::debug!(stage = "pitch/pyin", ms = t_pitch.elapsed().as_millis());

    tracing::debug!(
        stage = "total",
        ms = t0.elapsed().as_millis(),
        dur_s = samples.len() as f64 / sample_rate as f64
    );

    AnalysisResult {
        temporal,
        spectral,
        loudness,
        pitch,
        voice,
        f0_contour,
        energy_envelope,
    }
}

/// Cheap analysis — loudness, spectral, temporal only.  Skips pyin (the 1400ms bottleneck)
/// and voice metrics. Use for batch corpus fingerprinting; use `analyse()` for single-file detail.
pub fn analyse_fast(samples: &[f32], sample_rate: u32) -> AnalysisResult {
    let loudness = loudness::compute(samples, sample_rate);
    let energy_envelope = loudness::energy_envelope(samples, sample_rate);
    let spectral = spectral::compute(samples, sample_rate);
    let temporal = temporal::compute(samples, sample_rate);
    AnalysisResult {
        temporal,
        spectral,
        loudness,
        pitch: pitch::PitchMetrics::default(),
        voice: voice::VoiceMetrics::default(),
        f0_contour: vec![],
        energy_envelope,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_1s(freq: f32) -> Vec<f32> {
        (0..16000)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / 16000.0).sin() * 0.5)
            .collect()
    }

    #[test]
    fn analyse_returns_valid_metrics_for_sine() {
        let result = analyse(&sine_1s(440.0), 16000);
        assert!(result.loudness.rms_db < 0.0);
        assert!(result.spectral.brightness_hz > 0.0);
        assert!(result.temporal.duration_secs > 0.9);
    }

    #[test]
    fn analyse_silence_has_low_rms() {
        let silence = vec![0.0; 16000];
        let result = analyse(&silence, 16000);
        assert!(result.loudness.rms_db < -60.0);
    }

    #[test]
    fn analyse_fast_skips_pitch() {
        let result = analyse_fast(&sine_1s(440.0), 16000);
        assert_eq!(result.pitch.pitch_hz, 0.0);
        assert!(result.f0_contour.is_empty());
        assert!(result.spectral.brightness_hz > 0.0);
    }

    #[test]
    fn analyse_fast_produces_energy_envelope() {
        let result = analyse_fast(&sine_1s(440.0), 16000);
        assert!(!result.energy_envelope.is_empty());
    }

    #[test]
    fn analyse_short_signal_does_not_panic() {
        let short = vec![0.1; 100];
        let _result = analyse(&short, 16000);
    }

    #[test]
    fn analyse_result_serializes_to_json() {
        let result = analyse(&sine_1s(440.0), 16000);
        let json = serde_json::to_string(&result);
        assert!(json.is_ok());
    }
}
