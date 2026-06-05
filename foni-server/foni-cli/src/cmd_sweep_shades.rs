//! Sweep the Chatterbox parameter space to discover perceptually distinct shades.
//!
//! 1. Generate a grid of (exaggeration × cfg_weight × temperature) combos
//! 2. Synthesize the same text with each combo
//! 3. Analyze acoustic features (pitch variance, RMS, speech rate, crest)
//! 4. Cluster by feature similarity
//! 5. Output discovered shades with names

use std::path::PathBuf;

use foni_analyse::{analyse_fast, decode_wav};
use tracing::info;

const SWEEP_TEXT: &str = "The Emperor protects, and we shall know no fear.";

struct GridPoint {
    exaggeration: f32,
    cfg_weight: f32,
    temperature: f32,
}

#[derive(Debug, Clone)]
struct SweepSample {
    exaggeration: f32,
    cfg_weight: f32,
    temperature: f32,
    pitch_var_hz: f32,
    rms_db: f32,
    speech_rate: f32,
    crest_db: f32,
    duration_secs: f32,
    wav_path: PathBuf,
}

fn build_grid(steps: usize) -> Vec<GridPoint> {
    let exagg_range = linspace(0.3, 1.7, steps);
    let cfg_range = linspace(0.1, 0.6, steps);
    let temp_range = linspace(0.3, 1.3, steps);

    let mut grid = Vec::with_capacity(steps * steps * steps);
    for &e in &exagg_range {
        for &c in &cfg_range {
            for &t in &temp_range {
                grid.push(GridPoint {
                    exaggeration: e,
                    cfg_weight: c,
                    temperature: t,
                });
            }
        }
    }
    grid
}

fn linspace(start: f32, end: f32, steps: usize) -> Vec<f32> {
    if steps <= 1 {
        return vec![(start + end) / 2.0];
    }
    (0..steps)
        .map(|i| start + (end - start) * i as f32 / (steps - 1) as f32)
        .collect()
}

pub fn cmd_sweep_shades(
    server: &str,
    steps: usize,
    out_dir: &std::path::Path,
) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("mkdir: {e}"))?;

    let grid = build_grid(steps);
    info!(
        combos = grid.len(),
        steps,
        text = SWEEP_TEXT,
        "sweeping parameter space"
    );

    let client = foni_client::FoniClient::new(server);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio: {e}"))?;

    let mut samples = Vec::new();

    for (i, pt) in grid.iter().enumerate() {
        let label = format!(
            "e{:.1}_c{:.1}_t{:.1}",
            pt.exaggeration, pt.cfg_weight, pt.temperature
        );
        let wav_path = out_dir.join(format!("{label}.wav"));

        info!(
            combo = i + 1,
            total = grid.len(),
            exaggeration = format!("{:.1}", pt.exaggeration),
            cfg_weight = format!("{:.1}", pt.cfg_weight),
            temperature = format!("{:.1}", pt.temperature),
            "synthesizing"
        );

        let mut req = foni_client::SynthRequest::new(SWEEP_TEXT);
        req.voice = "en".into();
        req.dsp = false;
        req.exaggeration = Some(pt.exaggeration);
        req.cfg_weight = Some(pt.cfg_weight);
        req.temperature = Some(pt.temperature);

        let wav_data = rt
            .block_on(client.synthesize(&req))
            .map_err(|e| format!("synth {label}: {e}"))?;

        std::fs::write(&wav_path, &wav_data.0).map_err(|e| format!("write: {e}"))?;

        let wav = decode_wav(&wav_data.0).map_err(|e| format!("decode {label}: {e}"))?;
        let analysis = analyse_fast(&wav.samples, wav.sample_rate);

        samples.push(SweepSample {
            exaggeration: pt.exaggeration,
            cfg_weight: pt.cfg_weight,
            temperature: pt.temperature,
            pitch_var_hz: analysis.pitch.pitch_variation_hz,
            rms_db: analysis.loudness.rms_db,
            speech_rate: analysis.temporal.speech_rate,
            crest_db: analysis.loudness.crest_factor,
            duration_secs: analysis.temporal.duration_secs,
            wav_path,
        });
    }

    // Print feature matrix
    println!("exagg,cfg,temp,pitch_var,rms,rate,crest,duration");
    for s in &samples {
        println!(
            "{:.2},{:.2},{:.2},{:.1},{:.1},{:.1},{:.1},{:.2}",
            s.exaggeration,
            s.cfg_weight,
            s.temperature,
            s.pitch_var_hz,
            s.rms_db,
            s.speech_rate,
            s.crest_db,
            s.duration_secs
        );
    }

    // Cluster by normalized feature distance
    let clusters = cluster_samples(&samples);
    info!(clusters = clusters.len(), "distinct shade regions found");

    println!(
        "\n# Discovered shades ({} clusters from {} samples)",
        clusters.len(),
        samples.len()
    );
    for (i, centroid) in clusters.iter().enumerate() {
        let name = auto_name(centroid);
        println!(
            "shade(\"{name}\", &[(\"exaggeration\", {:.2}), (\"cfg_weight\", {:.2}), (\"temperature\", {:.2})]),",
            centroid.exaggeration, centroid.cfg_weight, centroid.temperature
        );
        info!(
            cluster = i + 1,
            name,
            exaggeration = format!("{:.2}", centroid.exaggeration),
            cfg_weight = format!("{:.2}", centroid.cfg_weight),
            temperature = format!("{:.2}", centroid.temperature),
            pitch_var = format!("{:.1}", centroid.pitch_var_hz),
            rms = format!("{:.1}", centroid.rms_db),
            "shade"
        );
    }

    Ok(())
}

fn cluster_samples(samples: &[SweepSample]) -> Vec<SweepSample> {
    if samples.is_empty() {
        return vec![];
    }

    // Normalize features to 0-1
    let features: Vec<[f32; 4]> = samples
        .iter()
        .map(|s| [s.pitch_var_hz, s.rms_db, s.speech_rate, s.crest_db])
        .collect();

    let mut mins = [f32::MAX; 4];
    let mut maxs = [f32::MIN; 4];
    for f in &features {
        for (i, &v) in f.iter().enumerate() {
            mins[i] = mins[i].min(v);
            maxs[i] = maxs[i].max(v);
        }
    }

    let norm: Vec<[f32; 4]> = features
        .iter()
        .map(|f| {
            let mut n = [0.0f32; 4];
            for i in 0..4 {
                let range = maxs[i] - mins[i];
                n[i] = if range > 1e-6 {
                    (f[i] - mins[i]) / range
                } else {
                    0.5
                };
            }
            n
        })
        .collect();

    // Simple greedy clustering: pick first sample, then add samples that are
    // far enough from all existing centroids
    let threshold = 0.25; // minimum normalized distance to be a new cluster
    let mut centroids: Vec<usize> = vec![0];

    for i in 1..norm.len() {
        let min_dist = centroids
            .iter()
            .map(|&c| euclidean(&norm[i], &norm[c]))
            .fold(f32::MAX, f32::min);

        if min_dist > threshold {
            centroids.push(i);
        }
    }

    centroids.iter().map(|&i| samples[i].clone()).collect()
}

fn euclidean(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

fn auto_name(s: &SweepSample) -> String {
    let energy = if s.exaggeration > 1.3 {
        "intense"
    } else if s.exaggeration > 0.8 {
        "moderate"
    } else {
        "calm"
    };

    let tone = if s.temperature > 1.0 {
        "warm"
    } else if s.temperature > 0.6 {
        "neutral"
    } else {
        "cold"
    };

    let grip = if s.cfg_weight < 0.2 {
        "commanding"
    } else if s.cfg_weight < 0.4 {
        "firm"
    } else {
        "loose"
    };

    format!("{energy}_{tone}_{grip}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linspace_produces_correct_steps() {
        let v = linspace(0.0, 1.0, 5);
        assert_eq!(v.len(), 5);
        assert!((v[0] - 0.0).abs() < 0.01);
        assert!((v[4] - 1.0).abs() < 0.01);
        assert!((v[2] - 0.5).abs() < 0.01);
    }

    #[test]
    fn linspace_single_step() {
        let v = linspace(0.0, 1.0, 1);
        assert_eq!(v.len(), 1);
        assert!((v[0] - 0.5).abs() < 0.01);
    }

    #[test]
    fn grid_size_matches() {
        assert_eq!(build_grid(3).len(), 27);
        assert_eq!(build_grid(4).len(), 64);
    }

    #[test]
    fn grid_covers_range() {
        let grid = build_grid(3);
        let min_e = grid.iter().map(|p| p.exaggeration).fold(f32::MAX, f32::min);
        let max_e = grid.iter().map(|p| p.exaggeration).fold(f32::MIN, f32::max);
        assert!((min_e - 0.3).abs() < 0.01);
        assert!((max_e - 1.7).abs() < 0.01);
    }

    #[test]
    fn euclidean_same_is_zero() {
        let a = [0.5, 0.5, 0.5, 0.5];
        assert!((euclidean(&a, &a)).abs() < 1e-6);
    }

    #[test]
    fn euclidean_different_is_positive() {
        let a = [0.0, 0.0, 0.0, 0.0];
        let b = [1.0, 1.0, 1.0, 1.0];
        assert!(euclidean(&a, &b) > 1.9);
    }

    #[test]
    fn cluster_deduplicates_identical() {
        let s = SweepSample {
            exaggeration: 0.5,
            cfg_weight: 0.3,
            temperature: 0.8,
            pitch_var_hz: 30.0,
            rms_db: -20.0,
            speech_rate: 3.0,
            crest_db: 10.0,
            duration_secs: 2.0,
            wav_path: PathBuf::new(),
        };
        let samples = vec![s.clone(), s.clone(), s.clone()];
        let clusters = cluster_samples(&samples);
        assert_eq!(clusters.len(), 1);
    }

    #[test]
    fn cluster_keeps_distinct() {
        let make = |pitch, rms| SweepSample {
            exaggeration: 0.5,
            cfg_weight: 0.3,
            temperature: 0.8,
            pitch_var_hz: pitch,
            rms_db: rms,
            speech_rate: 3.0,
            crest_db: 10.0,
            duration_secs: 2.0,
            wav_path: PathBuf::new(),
        };
        let samples = vec![make(10.0, -30.0), make(80.0, -10.0)];
        let clusters = cluster_samples(&samples);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn auto_name_covers_extremes() {
        let calm = auto_name(&SweepSample {
            exaggeration: 0.3,
            cfg_weight: 0.5,
            temperature: 1.2,
            pitch_var_hz: 0.0,
            rms_db: 0.0,
            speech_rate: 0.0,
            crest_db: 0.0,
            duration_secs: 0.0,
            wav_path: PathBuf::new(),
        });
        assert!(calm.starts_with("calm_warm_loose"));

        let intense = auto_name(&SweepSample {
            exaggeration: 1.7,
            cfg_weight: 0.1,
            temperature: 0.3,
            pitch_var_hz: 0.0,
            rms_db: 0.0,
            speech_rate: 0.0,
            crest_db: 0.0,
            duration_secs: 0.0,
            wav_path: PathBuf::new(),
        });
        assert!(intense.starts_with("intense_cold_commanding"));
    }
}
