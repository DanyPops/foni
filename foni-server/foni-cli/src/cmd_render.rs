//! Render a manifest — synthesize each beat with its shade, concatenate to one file.

use std::path::{Path, PathBuf};
use std::process::Command;

use foni_synth::engine::expression_palette::{ChatterboxColorset, Colorset};
use serde::Deserialize;
use tracing::info;

#[derive(Deserialize)]
pub struct Manifest {
    pub title: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    pub beats: Vec<ManifestBeat>,
}

#[derive(Deserialize)]
pub struct ManifestBeat {
    pub shade: String,
    pub text: String,
}

fn default_voice() -> String {
    "en".into()
}

pub fn cmd_render(
    server: &str,
    manifest_path: &Path,
    out: &Path,
    play: bool,
) -> Result<(), String> {
    let raw = std::fs::read_to_string(manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest: Manifest =
        serde_json::from_str(&raw).map_err(|e| format!("parse manifest: {e}"))?;

    let colorset = ChatterboxColorset::default();
    info!(
        title = manifest.title,
        beats = manifest.beats.len(),
        voice = manifest.voice,
        "rendering manifest"
    );

    let client = foni_client::FoniClient::new(server);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio: {e}"))?;
    let mut beat_paths = Vec::new();

    for (i, beat) in manifest.beats.iter().enumerate() {
        let shade = colorset.resolve(&beat.shade);
        let (exagg, cfg, temp) = match &shade {
            Some(s) => (
                s.params.get("exaggeration").copied().unwrap_or(0.5),
                s.params.get("cfg_weight").copied().unwrap_or(0.5),
                s.params.get("temperature").copied().unwrap_or(0.8),
            ),
            None => {
                tracing::warn!(shade = beat.shade, "unknown shade, using defaults");
                (0.5, 0.5, 0.8)
            }
        };

        info!(
            beat = i + 1,
            shade = beat.shade,
            text = truncate(&beat.text, 40),
            "synth"
        );

        let mut req = foni_client::SynthRequest::new(&beat.text);
        req.voice = manifest.voice.clone();
        req.dsp = false;
        req.exaggeration = Some(exagg);
        req.cfg_weight = Some(cfg);
        req.temperature = Some(temp);

        let wav_data = rt
            .block_on(client.synthesize(&req))
            .map_err(|e| format!("synth beat {}: {e}", i + 1))?;

        let beat_path = std::env::temp_dir().join(format!("foni_render_{i:02}.wav"));
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
