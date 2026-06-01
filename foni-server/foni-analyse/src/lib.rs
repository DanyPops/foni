pub mod alignment;
pub mod contour;
pub mod ecapa;
pub mod gap;
pub mod loudness;
pub mod mcd;
pub mod mfcc;
pub mod pitch;
pub mod report;
pub mod speaker_sim;
pub mod spectral;
pub mod temporal;
pub mod timeline;
pub mod visqol;
pub mod voice;
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

/// Multi-vector comparison result: aggregate gap + spectral distance + contour + intelligibility.
#[derive(Debug, Clone, Serialize)]
pub struct ComparisonResult {
    /// 9-metric aggregate gap scorer.
    pub gap: GapResult,
    /// Mel-Cepstral Distortion in dB. < 6dB = good, < 4dB = excellent.
    pub mcd_db: f32,
    /// F0 contour Pearson correlation after DTW alignment. 1.0 = perfect match.
    pub f0_corr: f32,
    /// Energy envelope Pearson correlation after DTW alignment.
    pub energy_corr: f32,
    /// Word Error Rate (%) via Whisper round-trip. None if not available.
    pub wer_pct: Option<f32>,
    /// Speaker similarity score 0–1. None if not computed.
    pub speaker_sim: Option<f32>,
    /// ViSQOL MOS-LQO 1–5. None when WAV paths unavailable or files too short.
    pub visqol_mos: Option<f32>,
    /// ECAPA-TDNN cosine similarity vs reference [0–1]. None when ONNX absent.
    pub ecapa_sim: Option<f32>,
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
    let mcd_db = mcd::compute_mcd(ref_samples, syn_samples, sample_rate);
    let (f0_corr, energy_corr) = contour::compute_contour_correlations(
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
    let speaker_sim = {
        let ref_embed = speaker_sim::embed(ref_samples, sample_rate, "reference");
        let syn_embed = speaker_sim::embed(syn_samples, sample_rate, "synthesis");
        Some(speaker_sim::speaker_similarity(&ref_embed, &syn_embed))
    };

    let visqol_mos = match (ref_path, syn_path) {
        (Some(r), Some(s)) => visqol::score(r, s),
        _ => None,
    };

    let ecapa_sim = ecapa_session.and_then(|sess: &mut ort::session::Session| {
        let ref_16k = ecapa::to_16k(ref_samples, sample_rate);
        let syn_16k = ecapa::to_16k(syn_samples, sample_rate);
        let ref_emb = ecapa::extract(sess, &ref_16k)?;
        let syn_emb = ecapa::extract(sess, &syn_16k)?;
        Some(ecapa::cosine_sim(&ref_emb, &syn_emb))
    });

    ComparisonResult {
        gap,
        mcd_db,
        f0_corr,
        energy_corr,
        wer_pct,
        speaker_sim,
        visqol_mos,
        ecapa_sim,
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
