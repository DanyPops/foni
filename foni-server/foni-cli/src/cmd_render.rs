//! Render a manifest — synthesize each beat with its shade, concatenate to one file.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use foni_synth::engine::expression_palette::{ChatterboxColorset, Colorset};
use futures::future::join_all;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use tokio::sync::Semaphore;
use tracing::info;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Manifest {
    pub title: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    /// Voice model name — maps to training/models/<model>/reference.wav + lang.
    pub model: Option<String>,
    pub beats: Vec<ManifestBeat>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ManifestBeat {
    pub shade: String,
    pub text: String,
}

fn default_voice() -> String {
    "en".into()
}

/// A resolved beat — manifest beat with shade parameters looked up.
#[derive(Debug, Clone)]
pub struct ResolvedBeat {
    pub index: usize,
    pub text: String,
    pub shade_name: String,
    pub exaggeration: f32,
    pub cfg_weight: f32,
    pub temperature: f32,
}

/// Parse manifest and resolve all shade names to parameter values.
pub fn resolve_manifest(manifest: &Manifest) -> Vec<ResolvedBeat> {
    let colorset = ChatterboxColorset::default();
    manifest
        .beats
        .iter()
        .enumerate()
        .map(|(i, beat)| {
            let (e, c, t) = resolve_shade(&colorset, &beat.shade);
            ResolvedBeat {
                index: i,
                text: beat.text.clone(),
                shade_name: beat.shade.clone(),
                exaggeration: e,
                cfg_weight: c,
                temperature: t,
            }
        })
        .collect()
}

fn resolve_shade(colorset: &dyn Colorset, name: &str) -> (f32, f32, f32) {
    // 1. Named alias.
    if let Some(s) = colorset.resolve(name) {
        return (
            s.params.get("exaggeration").copied().unwrap_or(0.5),
            s.params.get("cfg_weight").copied().unwrap_or(0.5),
            s.params.get("temperature").copied().unwrap_or(0.8),
        );
    }
    // 2. Label arithmetic: "Intense+Loose+Hot" — any order, case-insensitive.
    if let Some(result) = foni_synth::engine::expression_palette::resolve_labels(name) {
        return result;
    }
    tracing::warn!(shade = name, "unknown shade, using defaults");
    (0.5, 0.5, 0.8)
}

pub fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse manifest: {e}"))
}

/// Synthesize `count` items concurrently, bounded by `concurrency`.
/// `task(i)` produces bytes for beat `i`. Results are returned in index order.
pub async fn collect_parallel<F, Fut>(
    count: usize,
    concurrency: usize,
    task: F,
) -> Result<Vec<Vec<u8>>, String>
where
    F: Fn(usize) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Vec<u8>, String>> + Send + 'static,
{
    let sem = Arc::new(Semaphore::new(concurrency.max(1)));
    let task = Arc::new(task);

    let handles: Vec<_> = (0..count)
        .map(|i| {
            let sem = sem.clone();
            let task = task.clone();
            tokio::spawn(async move {
                let _permit = sem.acquire_owned().await.expect("semaphore closed");
                let bytes = task(i).await?;
                Ok::<(usize, Vec<u8>), String>((i, bytes))
            })
        })
        .collect();

    let mut indexed: Vec<(usize, Vec<u8>)> = join_all(handles)
        .await
        .into_iter()
        .map(|r| r.map_err(|e| format!("task panicked: {e}")).and_then(|r| r))
        .collect::<Result<_, _>>()?;

    indexed.sort_by_key(|(i, _)| *i);
    Ok(indexed.into_iter().map(|(_, b)| b).collect())
}

pub fn cmd_render(
    server: &str,
    manifest_path: &Path,
    out: &Path,
    play: bool,
    concurrency: usize,
) -> Result<(), String> {
    let manifest = load_manifest(manifest_path)?;
    let beats = resolve_manifest(&manifest);

    info!(
        title = manifest.title,
        beats = beats.len(),
        voice = manifest.voice,
        concurrency,
        "rendering manifest"
    );

    let pb = ProgressBar::new(beats.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len} beats  {msg}")
            .unwrap(),
    );

    let client = Arc::new(foni_client::FoniClient::new(server));
    let voice = manifest.voice.clone();
    let model = manifest.model.clone();
    let beats_arc = Arc::new(beats);
    let pb_clone = pb.clone();

    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio: {e}"))?;
    let wavs = rt.block_on(collect_parallel(beats_arc.len(), concurrency, move |i| {
        let client = client.clone();
        let beat = beats_arc[i].clone();
        let voice = voice.clone();
        let model = model.clone();
        let pb = pb_clone.clone();
        async move {
            let t0 = Instant::now();
            let mut req = foni_client::SynthRequest::new(&beat.text);
            req.voice = voice;
            req.model = model;
            req.dsp = false;
            req.exaggeration = Some(beat.exaggeration);
            req.cfg_weight = Some(beat.cfg_weight);
            req.temperature = Some(beat.temperature);

            let wav = client
                .synthesize(&req)
                .await
                .map_err(|e| format!("synth beat {}: {e}", beat.index + 1))?;

            pb.set_message(format!(
                "{} ({:.1}s)",
                beat.shade_name,
                t0.elapsed().as_secs_f32()
            ));
            pb.inc(1);
            info!(
                beat = beat.index + 1,
                shade = beat.shade_name,
                ms = t0.elapsed().as_millis() as u64,
                "done"
            );
            Ok(wav.0)
        }
    }))?;
    pb.finish_and_clear();

    let beat_paths: Vec<PathBuf> = wavs
        .iter()
        .enumerate()
        .map(|(i, wav)| {
            let p = std::env::temp_dir().join(format!("foni_render_{i:02}.wav"));
            std::fs::write(&p, wav).map_err(|e| format!("write beat {i}: {e}"))?;
            Ok(p)
        })
        .collect::<Result<_, String>>()?;

    concat_and_save(&beat_paths, out)?;

    if play {
        Command::new("paplay")
            .arg(out)
            .status()
            .map_err(|e| format!("paplay: {e}"))?;
    }

    Ok(())
}

fn concat_and_save(paths: &[PathBuf], out: &Path) -> Result<(), String> {
    if paths.len() == 1 {
        std::fs::copy(&paths[0], out).map_err(|e| format!("copy: {e}"))?;
        let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
        info!(beats = 1, size_kb = size / 1024, path = %out.display(), "rendered");
        return Ok(());
    }

    let mut args: Vec<String> = vec!["-y".into()];
    let mut filter = String::new();
    for (i, p) in paths.iter().enumerate() {
        args.extend(["-i".into(), p.to_string_lossy().into_owned()]);
        filter.push_str(&format!(
            "[{i}:a]aresample=24000,aformat=sample_fmts=s16:channel_layouts=mono[a{i}];"
        ));
    }
    for i in 0..paths.len() {
        filter.push_str(&format!("[a{i}]"));
    }
    filter.push_str(&format!("concat=n={}:v=0:a=1[out]", paths.len()));
    args.extend([
        "-filter_complex".into(),
        filter,
        "-map".into(),
        "[out]".into(),
        out.to_string_lossy().into_owned(),
    ]);

    let status = Command::new("ffmpeg")
        .args(&args)
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;

    if !status.success() {
        return Err("ffmpeg concat failed".into());
    }

    let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    info!(
        beats = paths.len(),
        size_kb = size / 1024,
        path = %out.display(),
        "rendered"
    );
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    // ── collect_parallel tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn parallel_collects_all_beats() {
        let out = collect_parallel(
            5,
            3,
            |i| async move { Ok::<Vec<u8>, String>(vec![i as u8]) },
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 5);
    }

    #[tokio::test]
    async fn parallel_preserves_index_order() {
        // Beats complete in reverse order (beat N-1 finishes first).
        // Output must still be sorted by original index.
        let out = collect_parallel(5, 5, |i| async move {
            let delay = (5 - i) as u64 * 10;
            tokio::time::sleep(Duration::from_millis(delay)).await;
            Ok::<Vec<u8>, String>(vec![i as u8])
        })
        .await
        .unwrap();
        for (idx, bytes) in out.iter().enumerate() {
            assert_eq!(bytes[0], idx as u8, "beat {idx} is out of order");
        }
    }

    #[tokio::test]
    async fn parallel_respects_concurrency_limit() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let limit = 3;

        collect_parallel(10, limit, {
            let active = active.clone();
            let peak = peak.clone();
            move |_| {
                let active = active.clone();
                let peak = peak.clone();
                async move {
                    let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(n, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok::<Vec<u8>, String>(vec![0])
                }
            }
        })
        .await
        .unwrap();

        assert!(
            peak.load(Ordering::SeqCst) <= limit,
            "peak concurrency {} exceeded limit {limit}",
            peak.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn parallel_propagates_failure() {
        let result = collect_parallel(5, 3, |i| async move {
            if i == 2 {
                Err("beat 2 exploded".to_string())
            } else {
                Ok::<Vec<u8>, String>(vec![0])
            }
        })
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("beat 2 exploded"));
    }

    #[tokio::test]
    async fn parallel_single_beat() {
        let out = collect_parallel(1, 5, |_| async move { Ok::<Vec<u8>, String>(vec![42]) })
            .await
            .unwrap();
        assert_eq!(out, vec![vec![42]]);
    }

    #[tokio::test]
    async fn parallel_zero_beats_returns_empty() {
        let out = collect_parallel(0, 5, |_| async move { Ok::<Vec<u8>, String>(vec![]) })
            .await
            .unwrap();
        assert!(out.is_empty());
    }

    fn test_manifest() -> Manifest {
        serde_json::from_str(
            r#"{
            "title": "Test",
            "voice": "en",
            "beats": [
                { "shade": "whisper", "text": "Quiet." },
                { "shade": "commanding", "text": "Loud!" },
                { "shade": "warm", "text": "Friendly." }
            ]
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn parse_manifest_json() {
        let m = test_manifest();
        assert_eq!(m.title, "Test");
        assert_eq!(m.voice, "en");
        assert_eq!(m.beats.len(), 3);
        assert_eq!(m.beats[0].shade, "whisper");
    }

    #[test]
    fn default_voice_is_english() {
        let m: Manifest =
            serde_json::from_str(r#"{"title": "T", "beats": [{"shade": "warm", "text": "hi"}]}"#)
                .unwrap();
        assert_eq!(m.voice, "en");
    }

    #[test]
    fn resolve_maps_known_shades() {
        let m = test_manifest();
        let beats = resolve_manifest(&m);
        assert_eq!(beats.len(), 3);

        assert_eq!(beats[0].shade_name, "whisper");
        assert!(beats[0].exaggeration < 0.4, "whisper should be low energy");

        assert_eq!(beats[1].shade_name, "commanding");
        assert!(
            beats[1].exaggeration > 1.0,
            "commanding should be high energy"
        );
    }

    #[test]
    fn resolve_unknown_shade_uses_defaults() {
        let m: Manifest = serde_json::from_str(
            r#"{"title": "T", "beats": [{"shade": "nonexistent", "text": "x"}]}"#,
        )
        .unwrap();
        let beats = resolve_manifest(&m);
        assert_eq!(beats.len(), 1);
        assert!((beats[0].exaggeration - 0.5).abs() < 0.01);
    }

    #[test]
    fn resolve_preserves_order() {
        let m = test_manifest();
        let beats = resolve_manifest(&m);
        assert_eq!(beats[0].index, 0);
        assert_eq!(beats[1].index, 1);
        assert_eq!(beats[2].index, 2);
        assert_eq!(beats[0].text, "Quiet.");
        assert_eq!(beats[2].text, "Friendly.");
    }

    #[test]
    fn resolve_different_shades_different_params() {
        let m = test_manifest();
        let beats = resolve_manifest(&m);
        assert!(
            (beats[0].exaggeration - beats[1].exaggeration).abs() > 0.3,
            "whisper and commanding should differ"
        );
    }

    #[test]
    fn manifest_roundtrip() {
        let json = serde_json::to_string(&test_manifest()).unwrap();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.beats.len(), 3);
    }

    #[test]
    fn load_manifest_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.json");
        std::fs::write(
            &path,
            r#"{"title": "F", "beats": [{"shade": "firm", "text": "ok"}]}"#,
        )
        .unwrap();
        let m = load_manifest(&path).unwrap();
        assert_eq!(m.title, "F");
        assert_eq!(m.beats[0].shade, "firm");
    }

    #[test]
    fn load_manifest_invalid_json_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(load_manifest(&path).is_err());
    }

    #[test]
    fn load_manifest_missing_file_errors() {
        assert!(load_manifest(Path::new("/nonexistent.json")).is_err());
    }

    #[test]
    fn concat_single_beat() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("a.wav");
        write_test_wav(&wav, 500);
        let out = dir.path().join("out.wav");
        concat_and_save(&[wav], &out).unwrap();
        assert!(out.exists());
    }

    #[test]
    fn concat_multiple_beats() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.wav");
        let b = dir.path().join("b.wav");
        write_test_wav(&a, 500);
        write_test_wav(&b, 300);
        let out = dir.path().join("out.wav");
        concat_and_save(&[a, b], &out).unwrap();
        assert!(out.exists());
        let bytes = std::fs::read(&out).unwrap();
        let wav = foni_analyse::decode_wav(&bytes).unwrap();
        let dur = wav.samples.len() as f32 / wav.sample_rate as f32;
        assert!(dur > 0.7, "should be ~0.8s, got {dur:.2}s");
    }

    fn write_test_wav(path: &Path, duration_ms: u32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 24_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        let n = 24_000 * duration_ms / 1000;
        for i in 0..n {
            let s = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 24_000.0).sin() * 0.5;
            w.write_sample((s * 32767.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
}
