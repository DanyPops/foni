//! Render a manifest — synthesize each beat with its shade, concatenate to one file.

use std::path::{Path, PathBuf};
use std::process::Command;

use foni_synth::engine::expression_palette::{ChatterboxColorset, Colorset};
use serde::Deserialize;
use tracing::info;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Manifest {
    pub title: String,
    #[serde(default = "default_voice")]
    pub voice: String,
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
    match colorset.resolve(name) {
        Some(s) => (
            s.params.get("exaggeration").copied().unwrap_or(0.5),
            s.params.get("cfg_weight").copied().unwrap_or(0.5),
            s.params.get("temperature").copied().unwrap_or(0.8),
        ),
        None => {
            tracing::warn!(shade = name, "unknown shade, using defaults");
            (0.5, 0.5, 0.8)
        }
    }
}

pub fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse manifest: {e}"))
}

pub fn cmd_render(
    server: &str,
    manifest_path: &Path,
    out: &Path,
    play: bool,
) -> Result<(), String> {
    let manifest = load_manifest(manifest_path)?;
    let beats = resolve_manifest(&manifest);

    info!(
        title = manifest.title,
        beats = beats.len(),
        voice = manifest.voice,
        "rendering manifest"
    );

    let client = foni_client::FoniClient::new(server);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio: {e}"))?;
    let mut beat_paths = Vec::new();

    for beat in &beats {
        info!(
            beat = beat.index + 1,
            shade = beat.shade_name,
            text = truncate(&beat.text, 40),
            "synth"
        );

        let mut req = foni_client::SynthRequest::new(&beat.text);
        req.voice = manifest.voice.clone();
        req.dsp = false;
        req.exaggeration = Some(beat.exaggeration);
        req.cfg_weight = Some(beat.cfg_weight);
        req.temperature = Some(beat.temperature);

        let wav_data = rt
            .block_on(client.synthesize(&req))
            .map_err(|e| format!("synth beat {}: {e}", beat.index + 1))?;

        let beat_path = std::env::temp_dir().join(format!("foni_render_{:02}.wav", beat.index));
        std::fs::write(&beat_path, &wav_data.0).map_err(|e| format!("write: {e}"))?;
        beat_paths.push(beat_path);
    }

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
