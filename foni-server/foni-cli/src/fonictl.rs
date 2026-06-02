// fonictl — foni-synth command-line factory.
// Commands: synth | studio | samples | listen | mix | status | play | analyse

mod tui;

use std::{path::PathBuf, process::Command};

use clap::{Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Input, Select};
use indicatif::{ProgressBar, ProgressStyle};

// ─── CLI definition ───────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "fonictl", about = "foni-synth WAV factory", version)]
struct Cli {
    /// foni-synth base URL
    #[arg(long, env = "FONI_SYNTH_URL", default_value = "http://localhost:5051")]
    server: String,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Synthesize text → WAV
    Synth {
        text: String,
        /// Play immediately after synthesis
        #[arg(short, long)]
        play: bool,
        /// Save to file (default: temp file)
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Model name
        #[arg(short, long, default_value = "sidorovich")]
        model: String,
        /// espeak voice
        #[arg(long, default_value = "ru")]
        voice: String,
        /// espeak speed (WPM)
        #[arg(long, default_value_t = 150)]
        speed: u32,
        /// Skip DSP chain
        #[arg(long)]
        no_dsp: bool,
        // DSP knobs
        #[arg(long)]
        rms_target_lufs: Option<f32>,
        #[arg(long)]
        compression_ratio: Option<f32>,
        #[arg(long)]
        tilt_low_db: Option<f32>,
        #[arg(long)]
        tilt_high_db: Option<f32>,
        #[arg(long)]
        vibrato_freq: Option<f32>,
        #[arg(long)]
        vibrato_depth: Option<f32>,
        #[arg(long)]
        presence_db: Option<f32>,
        #[arg(long)]
        de_ess_db: Option<f32>,
    },

    /// Maquette studio — produce N named variants, render all, listen, pick
    Studio {
        /// Phrase to synthesize for all maquettes
        #[arg(default_value = "Подойди-ка, надо тебе ситуацию прояснить.")]
        text: String,
        #[arg(short, long, default_value = "sidorovich")]
        model: String,
        /// Load maquettes from a JSON file instead of starting with defaults
        #[arg(long)]
        from: Option<PathBuf>,
    },

    /// Batch-generate comparison set (espeak / RVC / RVC+DSP)
    Samples {
        /// Output directory
        #[arg(short, long, default_value = "samples")]
        out_dir: PathBuf,
        #[arg(short, long, default_value = "sidorovich")]
        model: String,
    },

    /// Interactive DSP mixer REPL — play, tweak, compare, render
    Mix {
        /// Phrase to mix
        #[arg(default_value = "Подойди-ка, надо тебе ситуацию прояснить.")]
        text: String,
        #[arg(short, long, default_value = "sidorovich")]
        model: String,
        /// Load maquette presets from JSON instead of built-in defaults
        #[arg(long)]
        from: Option<PathBuf>,
        /// Play this WAV before each track for A/B reference (e.g. studio original)
        #[arg(long)]
        reference: Option<PathBuf>,
    },

    /// Print server health and loaded model
    Status,

    /// Render pipeline stages or DSP variants and play interactively
    Listen {
        /// Phrase to synthesize
        #[arg(default_value = "Подойди-ка, надо тебе ситуацию прояснить.")]
        text: String,
        /// Model name
        #[arg(short, long, default_value = "sidorovich")]
        model: String,
        /// Compare DSP variants (baseline/warm/punchy/bright) instead of pipeline stages
        #[arg(long)]
        dsp: bool,
        /// Synthesize RVC base once, then fan out through DSP isolation variants to find noise sources
        #[arg(long)]
        diagnose: bool,
        /// Play reference original before each stage (needs baseline/stalker/wav/sidorovich/trader1a.wav)
        #[arg(long)]
        vs: bool,
    },

    /// Apply DSP processing to a WAV file and write the result
    Process {
        /// Input WAV file
        file: PathBuf,
        /// Output WAV file (default: overwrites input with .processed.wav suffix)
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// DSP options as JSON object, e.g. '{"tiltLowDb":8,"rmsTargetLufs":-14}'
        #[arg(long, default_value = "{}")]
        opts: String,
        /// Also analyse result vs reference WAV
        #[arg(long)]
        vs: Option<PathBuf>,
    },

    /// Play a WAV file via system player
    Play { file: PathBuf },

    /// Print acoustic metrics for a WAV file
    Analyse {
        file: PathBuf,
        /// Compare against reference WAV
        #[arg(long)]
        vs: Option<PathBuf>,
        /// Show per-frame spectral distance timeline (requires --vs)
        #[arg(long)]
        timeline: bool,
    },

    /// Batch A/B/C/N tuning — run all presets through the compare pipeline, rank by gap
    /// Play maquette presets sequentially, hear reference then synthetic, rate each.
    /// With --auto N: run coordinate descent to find better DSP settings automatically.
    Tune {
        /// Phrase to synthesize for each preset
        #[arg(default_value = "Подойди-ка, надо тебе ситуацию прояснить.")]
        text: String,
        /// JSON file with named presets
        #[arg(long, default_value = "foni-maquettes.json")]
        presets: PathBuf,
        /// Reference WAV to play before each preset (A/B)
        #[arg(long)]
        reference: Option<PathBuf>,
        #[arg(short, long, default_value = "sidorovich")]
        model: String,
        /// Run N iterations of automatic knob search, save top-3 presets
        #[arg(long)]
        auto: Option<usize>,
        /// Reference WAV for gap analysis during auto-tuning
        #[arg(long)]
        vs: Option<PathBuf>,
    },

    /// 1:1 studio vs synthetic test harness
    Compare {
        /// Directory of studio WAV files (the ground truth)
        studio: PathBuf,
        /// Where to write synthetic WAVs (default: /tmp/fonictl_synthetic)
        #[arg(long, default_value = "/tmp/fonictl_synthetic")]
        out_dir: PathBuf,
        /// Only process WAVs shorter than this (seconds) — skips monologues
        #[arg(long, default_value_t = 8.0)]
        max_dur: f32,
        /// espeak voice / RVC model
        #[arg(short, long, default_value = "sidorovich")]
        model: String,
        /// Skip transcription, use existing .txt files in out_dir
        #[arg(long)]
        skip_transcribe: bool,
    },

    /// Acoustic fingerprint across a directory of WAV files — single Rust process
    Corpus {
        /// Directory of WAV files to aggregate
        dir: PathBuf,
        /// Compare aggregate against a reference WAV
        #[arg(long)]
        vs: Option<PathBuf>,
    },

    /// Measure how each DSP knob affects each acoustic metric — print the sensitivity matrix
    Calibrate {
        /// Phrase to synthesize for calibration
        #[arg(default_value = "Подойди-ка, надо тебе ситуацию прояснить.")]
        text: String,
        /// Reference WAV for target metrics
        #[arg(long)]
        vs: PathBuf,
        /// RVC model name
        #[arg(short, long, default_value = "sidorovich")]
        model: String,
    },

    /// Change one knob, re-synthesize, show before/after spectral diff
    Diff {
        /// Knob name (tiltHighDb, rmsTargetLufs, presenceDb, compressionRatio, deHarshDb, etc.)
        knob: String,
        /// New value for the knob
        #[arg(allow_negative_numbers = true)]
        value: f32,
        /// Phrase to synthesize
        #[arg(default_value = "Подойди-ка, надо тебе ситуацию прояснить.")]
        text: String,
        /// Reference WAV
        #[arg(long)]
        vs: PathBuf,
        /// RVC model name
        #[arg(short, long, default_value = "sidorovich")]
        model: String,
    },
}

// ─── HTTP helpers ─────────────────────────────────────────────────────────────

fn synth_request(
    server: &str,
    text: &str,
    model: &str,
    voice: &str,
    speed: u32,
    dsp: bool,
    opts: serde_json::Value,
) -> Result<Vec<u8>, String> {
    let mut body = serde_json::json!({
        "text":  text,
        "model": model,
        "voice": voice,
        "speed": speed,
        "dsp":   dsp,
        "opts":  opts,
    });

    // Merge opts into body.opts
    if let Some(o) = body.get_mut("opts") {
        *o = opts.clone();
    }

    let resp = reqwest::blocking::Client::new()
        .post(format!("{server}/synthesize"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "HTTP {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        ));
    }

    resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
}

/// POST /process — apply DSP to an existing WAV (base64 round-trip).
fn process_request(server: &str, wav: &[u8], opts: serde_json::Value) -> Result<Vec<u8>, String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let body = serde_json::json!({
        "audio_data": B64.encode(wav),
        "opts":       opts,
    });
    let resp = reqwest::blocking::Client::new()
        .post(format!("{server}/process"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .map_err(|e| format!("/process request: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "HTTP {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        ));
    }
    let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
    let b64 = data["audio_data"]
        .as_str()
        .ok_or("no audio_data in response")?;
    B64.decode(b64).map_err(|e| e.to_string())
}

fn get_json(server: &str, path: &str) -> Result<serde_json::Value, String> {
    reqwest::blocking::get(format!("{server}{path}"))
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())
}

// ─── Audio playback ───────────────────────────────────────────────────────────

/// Read sample count and sample rate from a WAV header (bytes 24–27 = sr, 40–43 = data size).
fn wav_duration_secs(path: &std::path::Path) -> Option<f64> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 44 {
        return None;
    }
    let sr = u32::from_le_bytes(data[24..28].try_into().ok()?) as f64;
    let channels = u16::from_le_bytes(data[22..24].try_into().ok()?) as f64;
    let bits = u16::from_le_bytes(data[34..36].try_into().ok()?) as f64;
    let data_size = u32::from_le_bytes(data[40..44].try_into().ok()?) as f64;
    Some(data_size / (sr * channels * bits / 8.0))
}

fn play_wav(path: &std::path::Path) {
    // paplay first: it drains the PipeWire/PulseAudio buffer before exiting,
    // so the process blocks until playback is actually done.
    // aplay exits as soon as audio is submitted to PipeWire — playback continues
    // async, causing the next prompt to appear mid-audio.
    for player in &["paplay", "afplay", "mpv", "aplay", "ffplay"] {
        if Command::new(player)
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return;
        }
    }
    // Last resort: calculate duration from WAV header and sleep.
    if let Some(dur) = wav_duration_secs(path) {
        std::thread::sleep(std::time::Duration::from_secs_f64(dur));
    }
    eprintln!("⚠  No audio player found (tried paplay, afplay, mpv, aplay, ffplay)");
    eprintln!("   File saved to: {}", path.display());
}

fn save_and_maybe_play(bytes: &[u8], out: Option<&PathBuf>, play: bool) -> PathBuf {
    let path = if let Some(p) = out {
        std::fs::write(p, bytes).unwrap();
        p.clone()
    } else {
        let mut tmp = std::env::temp_dir();
        tmp.push(format!(
            "fonictl_{}.wav",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::write(&tmp, bytes).unwrap();
        tmp
    };

    if play {
        play_wav(&path);
    }
    path
}

// ─── Subcommand handlers ──────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn cmd_synth(
    server: &str,
    text: &str,
    model: &str,
    voice: &str,
    speed: u32,
    dsp: bool,
    out: Option<&PathBuf>,
    play: bool,
    rms: Option<f32>,
    comp: Option<f32>,
    tilt_lo: Option<f32>,
    tilt_hi: Option<f32>,
    vibf: Option<f32>,
    vibd: Option<f32>,
    pres: Option<f32>,
    deess: Option<f32>,
) {
    let mut opts = serde_json::json!({});
    if let Some(v) = rms {
        opts["rmsTargetLufs"] = v.into();
    }
    if let Some(v) = comp {
        opts["compressionRatio"] = v.into();
    }
    if let Some(v) = tilt_lo {
        opts["tiltLowDb"] = v.into();
    }
    if let Some(v) = tilt_hi {
        opts["tiltHighDb"] = v.into();
    }
    if let Some(v) = vibf {
        opts["vibratoFreq"] = v.into();
    }
    if let Some(v) = vibd {
        opts["vibratoDepth"] = v.into();
    }
    if let Some(v) = pres {
        opts["presenceDb"] = v.into();
    }
    if let Some(v) = deess {
        opts["deEssDb"] = v.into();
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    let preview: String = text.chars().take(40).collect();
    pb.set_message(format!("Synthesizing: «{preview}»"));
    pb.enable_steady_tick(std::time::Duration::from_millis(80));

    match synth_request(server, text, model, voice, speed, dsp, opts) {
        Ok(bytes) => {
            pb.finish_and_clear();
            let path = save_and_maybe_play(&bytes, out, play);
            println!("✅  {}  ({} kB)", path.display(), bytes.len() / 1024);
        }
        Err(e) => {
            pb.finish_and_clear();
            eprintln!("❌  {e}");
            std::process::exit(1);
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Maquette {
    name: String,
    opts: serde_json::Value,
}

impl Maquette {
    fn describe(&self) -> String {
        let o = &self.opts;
        let mut parts = vec![];
        if let Some(v) = o.get("rmsTargetLufs") {
            parts.push(format!("rms={:.0}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("compressionRatio") {
            parts.push(format!("comp={:.1}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("tiltLowDb") {
            parts.push(format!("lo={:.0}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("tiltHighDb") {
            parts.push(format!("hi={:.0}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("presenceDb") {
            parts.push(format!("pres={:.1}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("deEssDb") {
            parts.push(format!("deess={:.0}", v.as_f64().unwrap_or(0.0)));
        }
        if parts.is_empty() {
            "defaults".into()
        } else {
            parts.join("  ")
        }
    }
}

/// Load maquettes from a JSON file if provided, falling back to built-in defaults.
/// JSON format: [{"name": "label", "opts": { DSP fields… }}]
fn load_maquettes(path: Option<&std::path::Path>) -> Vec<Maquette> {
    let Some(p) = path else {
        return default_maquettes();
    };
    let raw = match std::fs::read_to_string(p) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  ⚠  cannot read maquettes file {}: {e}", p.display());
            return default_maquettes();
        }
    };
    let entries: Vec<serde_json::Value> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  ⚠  invalid maquettes JSON: {e}");
            return default_maquettes();
        }
    };
    entries
        .into_iter()
        .filter_map(|v| {
            Some(Maquette {
                name: v["name"].as_str()?.to_owned(),
                opts: v["opts"].clone(),
            })
        })
        .collect()
}

fn default_maquettes() -> Vec<Maquette> {
    vec![
        Maquette {
            name: "baseline".into(),
            opts: serde_json::json!({
            "rmsTargetLufs": -8, "compressionRatio": 4, "compressionMakeupDb": 5,
            "tiltLowDb": 10, "tiltHighDb": -8, "vibratoFreq": 6, "vibratoDepth": 0.003 }),
        },
        Maquette {
            name: "warm".into(),
            opts: serde_json::json!({
            "rmsTargetLufs": -8, "compressionRatio": 3, "compressionMakeupDb": 4,
            "tiltLowDb": 14, "tiltHighDb": -10, "vibratoFreq": 6, "vibratoDepth": 0.003 }),
        },
        Maquette {
            name: "punchy".into(),
            opts: serde_json::json!({
            "rmsTargetLufs": -6, "compressionRatio": 6, "compressionMakeupDb": 6,
            "tiltLowDb": 8,  "tiltHighDb": -6,  "vibratoFreq": 5, "vibratoDepth": 0.002 }),
        },
        Maquette {
            name: "bright".into(),
            opts: serde_json::json!({
            "rmsTargetLufs": -9, "compressionRatio": 3, "compressionMakeupDb": 3,
            "tiltLowDb": 6,  "tiltHighDb": -4,  "presenceDb": 2.0, "deEssDb": 3.0,
            "vibratoFreq": 6, "vibratoDepth": 0.003 }),
        },
    ]
}

/// Render one maquette → WAV bytes, save to tmp, return path.
fn render_maquette(server: &str, text: &str, model: &str, m: &Maquette) -> Result<PathBuf, String> {
    let bytes = synth_request(server, text, model, "ru", 150, true, m.opts.clone())?;
    let path =
        std::env::temp_dir().join(format!("fonictl_maquette_{}.wav", m.name.replace(' ', "_")));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn cmd_studio(server: &str, text: &str, model: &str, from: Option<&std::path::Path>) {
    let theme = ColorfulTheme::default();

    let mut maquettes: Vec<Maquette> = if let Some(path) = from {
        let raw = std::fs::read_to_string(path).expect("cannot read maquette file");
        serde_json::from_str(&raw).expect("invalid maquette JSON")
    } else {
        default_maquettes()
    };

    // Rendered WAVs: index → path
    let mut rendered: Vec<Option<PathBuf>> = vec![None; maquettes.len()];

    println!("\n🎛  Maquette Studio");
    println!("    Phrase: «{text}»");
    println!("    Model:  {model}\n");

    loop {
        // Print table
        println!("  {:>3}  {:<18}  Config", "#", "Name");
        println!("  {}", "─".repeat(70));
        for (i, m) in maquettes.iter().enumerate() {
            let rendered_mark = if rendered.get(i).and_then(|r| r.as_ref()).is_some() {
                "✓"
            } else {
                " "
            };
            println!(
                "  {rendered_mark}{:>2}  {:<18}  {}",
                i + 1,
                m.name,
                m.describe()
            );
        }
        println!();

        let actions = vec![
            "▶  Render all maquettes",
            "🔊 Play a maquette",
            "✚  Add new maquette (fork existing + tweak)",
            "✎  Rename a maquette",
            "🗑  Delete a maquette",
            "💾 Save maquettes to JSON",
            "🏆 Pick winner → print opts",
            "✗  Quit",
        ];

        let choice = Select::with_theme(&theme)
            .with_prompt("Action")
            .items(&actions)
            .default(0)
            .interact()
            .unwrap_or(7);

        match choice {
            0 => {
                // Render all
                rendered.resize(maquettes.len(), None);
                let pb = ProgressBar::new(maquettes.len() as u64);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("[{bar:30}] {pos}/{len}  {msg}")
                        .unwrap(),
                );
                for (i, m) in maquettes.iter().enumerate() {
                    pb.set_message(m.name.clone());
                    match render_maquette(server, text, model, m) {
                        Ok(path) => {
                            rendered[i] = Some(path);
                            pb.inc(1);
                        }
                        Err(e) => {
                            pb.println(format!("  ❌ {}: {e}", m.name));
                            pb.inc(1);
                        }
                    }
                }
                pb.finish_and_clear();
                println!("  ✅ All rendered\n");
            }
            1 => {
                // Play one
                let playable: Vec<(usize, &Maquette)> = maquettes
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| rendered.get(*i).and_then(|r| r.as_ref()).is_some())
                    .collect();
                if playable.is_empty() {
                    println!("  ⚠  No maquettes rendered yet — render first\n");
                    continue;
                }
                let labels: Vec<String> = playable
                    .iter()
                    .map(|(i, m)| format!("{:>2}. {:<18} {}", i + 1, m.name, m.describe()))
                    .collect();
                let sel = Select::with_theme(&theme)
                    .with_prompt("Play which maquette")
                    .items(&labels)
                    .interact()
                    .unwrap_or(0);
                let (idx, _) = playable[sel];
                if let Some(path) = &rendered[idx] {
                    println!("  ▶  Playing {}…", maquettes[idx].name);
                    play_wav(path);
                }
                println!();
            }
            2 => {
                // Add maquette (fork + tweak)
                let names: Vec<String> = maquettes.iter().map(|m| m.name.clone()).collect();
                let base_sel = Select::with_theme(&theme)
                    .with_prompt("Fork from")
                    .items(&names)
                    .interact()
                    .unwrap_or(0);

                let new_name: String = Input::with_theme(&theme)
                    .with_prompt("New maquette name")
                    .interact_text()
                    .unwrap_or_default();

                if new_name.is_empty() {
                    continue;
                }

                let mut new_opts = maquettes[base_sel].opts.clone();

                // Knob list to tweak
                let knob_defs = [
                    ("rmsTargetLufs", "Loudness target (dBLUFS)"),
                    ("compressionRatio", "Compression ratio (N:1)"),
                    ("compressionMakeupDb", "Makeup gain (dB)"),
                    ("tiltLowDb", "Low-shelf boost (dB)"),
                    ("tiltHighDb", "High-shelf cut (dB)"),
                    ("presenceDb", "Presence boost (dB)"),
                    ("deEssDb", "De-esser cut (dB)"),
                    ("vibratoFreq", "Vibrato rate (Hz)"),
                    ("vibratoDepth", "Vibrato depth"),
                ];

                loop {
                    println!("\n  Knobs (press Enter to keep, type new value to change):");
                    let mut done = false;
                    for (key, label) in &knob_defs {
                        let cur = new_opts.get(*key).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let val: String = Input::with_theme(&theme)
                            .with_prompt(format!("  {label} [{cur:.3}]"))
                            .allow_empty(true)
                            .interact_text()
                            .unwrap_or_default();
                        if val == "done" || val == "q" {
                            done = true;
                            break;
                        }
                        if !val.is_empty() {
                            if let Ok(f) = val.parse::<f64>() {
                                new_opts[*key] = f.into();
                            }
                        }
                    }
                    if done {
                        break;
                    }

                    // Offer to tweak again or confirm
                    let confirm = Select::with_theme(&theme)
                        .with_prompt("Confirm?")
                        .items(&["✓  Add this maquette", "✎  Tweak more", "✗  Cancel"])
                        .interact()
                        .unwrap_or(2);
                    match confirm {
                        0 => {
                            maquettes.push(Maquette {
                                name: new_name.clone(),
                                opts: new_opts.clone(),
                            });
                            rendered.push(None);
                            println!("  ✅ Added «{new_name}»\n");
                            break;
                        }
                        2 => break,
                        _ => {}
                    }
                }
            }
            3 => {
                // Rename
                let names: Vec<String> = maquettes.iter().map(|m| m.name.clone()).collect();
                let sel = Select::with_theme(&theme)
                    .with_prompt("Rename which")
                    .items(&names)
                    .interact()
                    .unwrap_or(0);
                let new_name: String = Input::with_theme(&theme)
                    .with_prompt("New name")
                    .interact_text()
                    .unwrap_or_default();
                if !new_name.is_empty() {
                    maquettes[sel].name = new_name;
                }
            }
            4 => {
                // Delete
                if maquettes.len() <= 1 {
                    println!("  ⚠  Cannot delete last maquette\n");
                    continue;
                }
                let names: Vec<String> = maquettes.iter().map(|m| m.name.clone()).collect();
                let sel = Select::with_theme(&theme)
                    .with_prompt("Delete which")
                    .items(&names)
                    .interact()
                    .unwrap_or(0);
                maquettes.remove(sel);
                rendered.remove(sel);
                println!("  ✅ Deleted\n");
            }
            5 => {
                // Save to JSON
                let path: String = Input::with_theme(&theme)
                    .with_prompt("Save to file")
                    .default("maquettes.json".into())
                    .interact_text()
                    .unwrap_or_default();
                if !path.is_empty() {
                    let json = serde_json::to_string_pretty(&maquettes).unwrap();
                    std::fs::write(&path, &json).unwrap();
                    println!("  ✅ Saved to {path}\n");
                }
            }
            6 => {
                // Pick winner
                let names: Vec<String> = maquettes.iter().map(|m| m.name.clone()).collect();
                let sel = Select::with_theme(&theme)
                    .with_prompt("Winner")
                    .items(&names)
                    .interact()
                    .unwrap_or(0);
                let winner = &maquettes[sel];
                println!("\n🏆  Winner: «{}»", winner.name);
                println!("{}", serde_json::to_string_pretty(&winner.opts).unwrap());
                println!("\n  Use with:");
                let args: Vec<String> = winner
                    .opts
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| {
                        let flag = k
                            .chars()
                            .flat_map(|c| {
                                if c.is_uppercase() {
                                    vec!['-', c.to_lowercase().next().unwrap()]
                                } else {
                                    vec![c]
                                }
                            })
                            .collect::<String>();
                        format!("--{flag} {v}")
                    })
                    .collect();
                println!("  fonictl synth \"{}\" {}", text, args.join(" \\\n    "));
                println!();
            }
            _ => break,
        }
    }
}

/// Synthesize RVC base once, then apply isolation DSP configs via /process.
/// Each stage removes one effect to identify noise sources.
fn cmd_diagnose(server: &str, text: &str, model: &str) {
    use std::io::{BufRead, Write};

    eprintln!("\n⚠  Diagnose — isolating noise sources");
    println!("   Phrase: «{text}»");
    eprintln!("   Step 1: synthesizing RVC base (no DSP) …");

    // Synthesize RVC without DSP once — this is the base for all variants.
    let rvc_wav = match synth_request(server, text, model, "ru", 150, false, serde_json::json!({}))
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("  ❌ RVC synthesis failed: {e}");
            return;
        }
    };
    eprintln!("   Base: {} kB", rvc_wav.len() / 1024);

    let full: serde_json::Value = serde_json::json!({
        "rmsTargetLufs": -8, "compressionRatio": 4, "compressionMakeupDb": 5,
        "tiltLowDb": 10,  "tiltHighDb": -8,
        "vibratoFreq": 6, "vibratoDepth": 0.003,
        "reverbMs": 8,    "reverbDecay": 0.04
    });

    let mut stages: Vec<(&str, &str, serde_json::Value)> = vec![
        // label, description, opts
        (
            "a_rvc_raw",
            "RVC only — no DSP at all",
            serde_json::json!({
                "compressionRatio": 1.0, "compressionMakeupDb": 0.0,
                "tiltLowDb": 0.0, "tiltHighDb": 0.0,
                "vibratoDepth": 0.0, "reverbMs": 0.0, "reverbDecay": 0.0,
                "rmsTargetLufs": 0.0, "normalize": false
            }),
        ),
        (
            "b_novibrato",
            "DSP full — vibrato OFF   ← if wobble disappears: vibrato is culprit",
            {
                let mut v = full.clone();
                v["vibratoDepth"] = 0.0.into();
                v
            },
        ),
        (
            "c_nocomp",
            "DSP — vibrato OFF + compressor OFF   ← buzz from dynamics?",
            {
                let mut v = full.clone();
                v["vibratoDepth"] = 0.0.into();
                v["compressionRatio"] = 1.0.into();
                v["compressionMakeupDb"] = 0.0.into();
                v
            },
        ),
        (
            "d_notilt",
            "DSP — vibrato OFF + tilt OFF   ← buzz from spectral tilt?",
            {
                let mut v = full.clone();
                v["vibratoDepth"] = 0.0.into();
                v["tiltLowDb"] = 0.0.into();
                v["tiltHighDb"] = 0.0.into();
                v
            },
        ),
        ("e_noreverb", "DSP — vibrato OFF + reverb OFF", {
            let mut v = full.clone();
            v["vibratoDepth"] = 0.0.into();
            v["reverbMs"] = 0.0.into();
            v["reverbDecay"] = 0.0.into();
            v
        }),
        (
            "f_full_dsp",
            "Full DSP (current state) — all effects ON",
            full.clone(),
        ),
    ];

    // Render all via /process
    println!(
        "   Step 2: applying {} DSP configs via /process …",
        stages.len()
    );
    let pb = ProgressBar::new(stages.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:30}] {pos}/{len}")
            .unwrap(),
    );

    struct Stage {
        label: String,
        desc: String,
        path: std::path::PathBuf,
    }
    let mut rendered: Vec<Stage> = Vec::new();

    for (label, desc, opts) in &stages {
        match process_request(server, &rvc_wav, opts.clone()) {
            Ok(wav) => {
                let path = std::env::temp_dir().join(format!("fonictl_diag_{label}.wav"));
                std::fs::write(&path, &wav).unwrap();
                rendered.push(Stage {
                    label: label.to_string(),
                    desc: desc.to_string(),
                    path,
                });
            }
            Err(e) => pb.println(format!("  ❌ {label}: {e}")),
        }
        pb.inc(1);
    }
    pb.finish_and_clear();

    println!("\n   Rendered files:");
    for s in &rendered {
        println!(
            "     fonictl play {}   # {}",
            s.path.display(),
            s.desc.split('—').next().unwrap_or("").trim()
        );
    }
    println!("\n   Controls: Enter=next  r=replay  p=prev  q=quit\n");

    let stdin = std::io::stdin();
    let mut i = 0usize;
    loop {
        if i >= rendered.len() {
            break;
        }
        let s = &rendered[i];
        println!("▶  [{}/{}] {}", i + 1, rendered.len(), s.label);
        println!("   {}", s.desc);
        play_wav(&s.path);
        print!("   > ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        stdin.lock().read_line(&mut line).ok();
        match line.trim() {
            "q" | "quit" => break,
            "r" | "replay" => {}
            "p" | "prev" => {
                i = i.saturating_sub(1);
            }
            _ => {
                i += 1;
            }
        }
    }
    println!("\n  done.");
}

fn cmd_process(
    server: &str,
    file: &PathBuf,
    out: Option<&PathBuf>,
    opts_str: &str,
    vs: Option<&PathBuf>,
) {
    use foni_analyse::{analyse, compute_gap, decode_wav, format_gap_table, TargetTensor};

    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {}: {e}", file.display());
            return;
        }
    };
    let opts: serde_json::Value = match serde_json::from_str(opts_str) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("invalid --opts JSON: {e}");
            return;
        }
    };

    let result = match process_request(server, &bytes, opts) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("process failed: {e}");
            return;
        }
    };

    let out_path = out.cloned().unwrap_or_else(|| {
        let stem = file.file_stem().unwrap_or_default().to_string_lossy();
        file.with_file_name(format!("{stem}.processed.wav"))
    });

    if let Err(e) = std::fs::write(&out_path, &result) {
        eprintln!("cannot write {}: {e}", out_path.display());
        return;
    }
    println!("{}", out_path.display());

    if let Some(ref_path) = vs {
        let ref_bytes = match std::fs::read(ref_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("cannot read reference: {e}");
                return;
            }
        };
        let ref_wav = decode_wav(&ref_bytes).expect("reference WAV");
        let syn_wav = decode_wav(&result).expect("processed WAV");
        let ref_analysis = analyse(&ref_wav.samples, ref_wav.sample_rate);
        let syn_analysis = analyse(&syn_wav.samples, syn_wav.sample_rate);
        let tensor = TargetTensor::from_analysis(&ref_analysis, &ref_path.display().to_string());
        let gap = compute_gap(&out_path.display().to_string(), &syn_analysis, &tensor);
        println!("{}", format_gap_table(&gap));
    }
}

fn cmd_listen(server: &str, text: &str, model: &str, dsp_variants: bool, play_ref: bool) {
    use std::io::{BufRead, Write};

    let ref_path = std::path::PathBuf::from("baseline/stalker/wav/sidorovich/trader1a.wav");

    // Stages: (label, prosody, dsp)
    let stages: &[(&str, bool, bool)] = if dsp_variants {
        &[
            ("baseline", true, true),
            ("warm", true, true),
            ("punchy", true, true),
            ("bright", true, true),
        ]
    } else {
        &[
            ("1 espeak raw", false, false),
            ("2 rvc", false, false),
            ("3 rvc+dsp", false, true),
            ("4 rvc+dsp+prosody", true, true),
        ]
    };

    // For pipeline mode, espeak stage is special (no RVC).
    println!("\n⚘  Listen — rendering {} stages", stages.len());
    println!("   Phrase: «{text}»\n");

    // Pre-render all stages.
    let pb = ProgressBar::new(stages.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:30}] {pos}/{len}  {msg}")
            .unwrap(),
    );

    let maquettes: Vec<Maquette> = default_maquettes();

    struct Stage {
        label: String,
        path: std::path::PathBuf,
    }
    let mut rendered: Vec<Stage> = Vec::new();

    for (i, (label, prosody, dsp)) in stages.iter().enumerate() {
        pb.set_message(label.to_string());

        let result: Result<Vec<u8>, String> = if i == 0 && !dsp_variants {
            // Stage 0: raw espeak only, bypass RVC entirely.
            let tmp = std::env::temp_dir().join("fonictl_espeak_raw.wav");
            let status = Command::new("espeak-ng")
                .args(["-v", "ru", "-s", "150", "-w"])
                .arg(&tmp)
                .arg(text)
                .status();
            match status {
                Ok(s) if s.success() => std::fs::read(&tmp).map_err(|e| e.to_string()),
                Ok(_) => Err("espeak-ng failed".into()),
                Err(e) => Err(e.to_string()),
            }
        } else {
            let opts = if dsp_variants {
                maquettes.get(i).map(|m| m.opts.clone()).unwrap_or_default()
            } else if *dsp {
                default_maquettes()[0].opts.clone()
            } else {
                serde_json::json!({})
            };
            synth_request(server, text, model, "ru", 150, *dsp, opts).map_err(|e| e)
        };

        match result {
            Ok(bytes) => {
                let path = std::env::temp_dir().join(format!("fonictl_listen_{i}.wav"));
                std::fs::write(&path, &bytes).unwrap();
                rendered.push(Stage {
                    label: label.to_string(),
                    path,
                });
                pb.inc(1);
            }
            Err(e) => {
                pb.println(format!("  ❌ {label}: {e}"));
                pb.inc(1);
            }
        }
    }
    pb.finish_and_clear();

    if rendered.is_empty() {
        println!("  No stages rendered.");
        return;
    }

    println!("  ✓  All stages ready.  Controls: Enter=next  r=replay  q=quit\n");

    let stdin = std::io::stdin();
    let mut i = 0usize;
    loop {
        if i >= rendered.len() {
            break;
        }
        let stage = &rendered[i];

        println!("▶  [{}] {}", i + 1, stage.label);

        if play_ref && ref_path.exists() {
            println!("   ▶ reference (Sidorovich original)");
            play_wav(&ref_path);
        }

        play_wav(&stage.path);

        print!("   [Enter] next  [r] replay  [p] prev  [q] quit  > ");
        std::io::stdout().flush().ok();

        let mut line = String::new();
        stdin.lock().read_line(&mut line).ok();
        match line.trim() {
            "q" | "quit" => break,
            "r" | "replay" => {} // replay same index
            "p" | "prev" => {
                i = i.saturating_sub(1);
            }
            _ => {
                i += 1;
            }
        }
    }

    println!("\n  done.");
}

// ─── Mixer REPL ────────────────────────────────────────────────────────────────────

// ─ Session file — the AI/human contract ──────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct TrackRecord {
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    rating: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    winner: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    opts: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    gap_pct: Option<f32>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct AiSuggestion {
    label: String,
    rationale: String,
    opts: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct MixerSession {
    phrase: String,
    model: String,
    saved_at: String,
    tracks: Vec<TrackRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ai_suggestions: Vec<AiSuggestion>,
}

impl MixerSession {
    fn path() -> std::path::PathBuf {
        let base = std::env::var("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let mut p = dirs_next("HOME");
                p.push(".local/share");
                p
            });
        let dir = base.join("foni");
        std::fs::create_dir_all(&dir).ok();
        dir.join("mixer-session.json")
    }

    fn save(&self) {
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(Self::path(), json).ok();
    }

    fn load() -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(Self::path()).ok()?).ok()
    }

    fn now() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (y, mo, d, h, mi, s) = epoch_ymd(secs);
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }
}

fn dirs_next(var: &str) -> std::path::PathBuf {
    std::env::var(var)
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
}

fn epoch_ymd(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let (s, m, h) = (secs % 60, (secs / 60) % 60, (secs / 3600) % 24);
    let mut rem = secs / 86400;
    let mut y = 1970u64;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let yd = if leap { 366 } else { 365 };
        if rem < yd {
            break;
        }
        rem -= yd;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let md = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 0u64;
    for &days in &md {
        if rem < days {
            break;
        }
        rem -= days;
        mo += 1;
    }
    (y, mo + 1, rem + 1, h, m, s)
}

// ─ Runtime track (in-memory only) ─────────────────────────────────────────────

struct Track {
    label: String,
    desc: String,
    path: std::path::PathBuf,
    opts: serde_json::Value,
    rating: Option<u8>,
    note: Option<String>,
    winner: bool,
}

fn stars(rating: Option<u8>) -> String {
    match rating {
        None => "     ".into(),
        Some(n) => format!("{}☆{}", "★".repeat(n as usize), "★".repeat(5 - n as usize)),
    }
}

fn mixer_print_tracks(tracks: &[Track]) {
    println!();
    println!(
        "  {:>3}  {:<20}  {:<7}  {}",
        "#", "Label", "Rating", "Config"
    );
    println!("  {}", "─".repeat(78));
    for (i, t) in tracks.iter().enumerate() {
        let dot = if t.path.exists() { "●" } else { "◦" };
        let crown = if t.winner { " ✪" } else { "  " };
        let rating = stars(t.rating);
        println!(
            "  {dot}{crown}{:>2}  {:<20}  {rating}  {}",
            i + 1,
            t.label,
            t.desc
        );
        if let Some(ref n) = t.note {
            println!("        └ {n}");
        }
    }
    println!();
}

fn opts_describe(opts: &serde_json::Value) -> String {
    let get = |k: &str| opts.get(k).and_then(|v| v.as_f64());
    let mut parts = Vec::new();
    if let Some(v) = get("rmsTargetLufs") {
        parts.push(format!("rms={v:.0}"))
    }
    if let Some(v) = get("compressionRatio") {
        parts.push(format!("comp={v:.1}"))
    }
    if let Some(v) = get("tiltLowDb") {
        parts.push(format!("lo={v:.0}"))
    }
    if let Some(v) = get("tiltHighDb") {
        parts.push(format!("hi={v:.0}"))
    }
    if let Some(v) = get("vibratoDepth") {
        parts.push(format!("vib={v:.4}"))
    }
    if let Some(v) = get("presenceDb") {
        parts.push(format!("pres={v:.1}"))
    }
    if let Some(v) = get("deEssDb") {
        parts.push(format!("deess={v:.0}"))
    }
    if let Some(v) = get("reverbMs") {
        parts.push(format!("rev={v:.0}ms"))
    }
    if parts.is_empty() {
        "defaults".into()
    } else {
        parts.join("  ")
    }
}

const MIXER_HELP: &str = r#"
  Playback:
    N            play track N
    r            replay last
    cmp A B      play A then B  ('ref' = Sidorovich original)

  Rating (written to ~/.local/share/foni/mixer-session.json):
    rate N SCORE note...   rate track N (1-5) with optional note
    winner N               mark track N as winner (auto-saves)
    note N TEXT            add/replace note on track N
    ai                     load AI suggestions as pending tracks
    save                   write session file now

  DSP scratchpad:
    opts                   show current scratchpad opts
    set KEY VAL            tweak a param  (e.g. set vibratoDepth 0)
    reset                  reset to defaults
    fork N                 copy track N opts to scratchpad
    render NAME            render scratchpad as new track

  Tracks:
    ls                     list all tracks
    drop N                 remove track N
    analyse N              acoustic metrics for track N
    phrase TEXT            change phrase (clears rendered tracks)

    q / quit               save session and exit
"#;

fn cmd_mix(
    server: &str,
    text: &str,
    model: &str,
    from: Option<&std::path::Path>,
    reference: Option<&std::path::Path>,
) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let maquettes = load_maquettes(from);

    // Seed tracks from maquettes (pre-render if WAV exists).
    let mut tracks: Vec<tui::state::Track> = maquettes
        .into_iter()
        .map(|m| {
            let path =
                std::env::temp_dir().join(format!("fonictl_mix_{}.wav", m.name.replace(' ', "_")));
            tui::state::Track {
                label: m.name.clone(),
                desc: m.describe(),
                path,
                opts: serde_json::from_value(m.opts).unwrap_or_default(),
                rating: None,
                note: None,
                winner: false,
                analyse: None,
            }
        })
        .collect();

    // Pick up existing diagnose WAVs from the last --diagnose run.
    for (slug, desc) in &[
        ("a_rvc_raw", "RVC only"),
        ("b_novibrato", "vibrato off"),
        ("c_nocomp", "no compression"),
        ("d_notilt", "no tilt"),
        ("e_noreverb", "no reverb"),
        ("f_full_dsp", "full DSP"),
    ] {
        let path = std::env::temp_dir().join(format!("fonictl_diag_{slug}.wav"));
        if path.exists() {
            tracks.push(tui::state::Track {
                label: slug.to_string(),
                desc: desc.to_string(),
                path,
                opts: Default::default(),
                rating: None,
                note: None,
                winner: false,
                analyse: None,
            });
        }
    }

    // Pre-render un-rendered maquettes upfront.
    {
        let to_render: Vec<usize> = tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.path.exists())
            .map(|(i, _)| i)
            .collect();

        if !to_render.is_empty() {
            eprintln!("  Rendering {} maquettes…", to_render.len());
            let pb = ProgressBar::new(to_render.len() as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("[{bar:30}] {pos}/{len}  {msg}")
                    .unwrap(),
            );

            let client = foni_client::FoniClient::new(server);
            let base_result = rt.block_on(client.synthesize(&foni_client::SynthRequest {
                text: text.to_string(),
                model: Some(model.to_string()),
                dsp: false,
                prosody: false,
                ..foni_client::SynthRequest::new(text)
            }));

            if let Ok(base) = base_result {
                for i in to_render {
                    let t = &tracks[i];
                    pb.set_message(t.label.clone());
                    if let Ok(wav) = rt.block_on(client.process(&base, t.opts.clone())) {
                        std::fs::write(&t.path, wav.as_bytes()).ok();
                    }
                    pb.inc(1);
                }
            } else {
                pb.println("  ⚠  server unreachable — tracks will render on demand");
            }
            pb.finish_and_clear();
        }
    }

    tui::run(
        &rt,
        server,
        text,
        model,
        tracks,
        reference.map(|p| p.to_path_buf()),
    );
}

fn save_session(tracks: &[Track], phrase: &str, model: &str) {
    let session = MixerSession {
        phrase: phrase.to_string(),
        model: model.to_string(),
        saved_at: MixerSession::now(),
        tracks: tracks
            .iter()
            .map(|t| TrackRecord {
                label: t.label.clone(),
                rating: t.rating,
                winner: if t.winner { Some(true) } else { None },
                note: t.note.clone(),
                opts: t.opts.clone(),
                gap_pct: None,
            })
            .collect(),
        ai_suggestions: Vec::new(),
    };
    session.save();
}

fn cmd_samples(server: &str, out_dir: &PathBuf, model: &str) {
    let phrases = vec![
        ("01_trader1a", "Подойди-ка, надо тебе ситуацию прояснить."),
        ("02_greeting", "Привет, сталкер. Как дела на болотах?"),
        ("03_warning", "Осторожно. Здесь аномалии, не зевай."),
        ("04_deal", "Деплой прошёл успешно, коммиты запушены."),
        ("05_farewell", "Удачи, браток. На Зоне удача нужна."),
    ];

    std::fs::create_dir_all(out_dir).unwrap();

    // Copy reference if present
    let ref_src = PathBuf::from("baseline/stalker/wav/sidorovich/trader1a.wav");
    if ref_src.exists() {
        let dst = out_dir.join("00_reference_original.wav");
        std::fs::copy(&ref_src, &dst).unwrap();
        println!("✅  00_reference_original.wav");
    }

    let default_opts = serde_json::json!({
        "rmsTargetLufs":       -8,
        "compressionRatio":     4,
        "compressionMakeupDb":  5,
        "tiltLowDb":           10,
        "tiltHighDb":          -8,
        "vibratoFreq":          6,
        "vibratoDepth":        0.003
    });

    let pb = ProgressBar::new((phrases.len() * 2) as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:30}] {pos}/{len} {msg}")
            .unwrap(),
    );

    for (slug, phrase) in &phrases {
        for (dsp, suffix) in &[(true, "b_rvc_dsp"), (false, "c_rvc_nodsp")] {
            pb.set_message(format!("{slug}_{suffix}"));
            let opts = if *dsp {
                default_opts.clone()
            } else {
                serde_json::json!({})
            };
            match synth_request(server, phrase, model, "ru", 150, *dsp, opts) {
                Ok(bytes) => {
                    let path = out_dir.join(format!("{slug}_{suffix}.wav"));
                    std::fs::write(&path, &bytes).unwrap();
                    pb.inc(1);
                }
                Err(e) => {
                    pb.println(format!("❌  {slug}_{suffix}: {e}"));
                    pb.inc(1);
                }
            }
        }
    }
    pb.finish_and_clear();

    println!("\n── Samples in {}", out_dir.display());
    println!("   00_reference_original.wav      ← studio");
    for (slug, _) in &phrases {
        println!("   {slug}_b_rvc_dsp.wav         ← RVC + DSP");
        println!("   {slug}_c_rvc_nodsp.wav       ← RVC only");
    }
}

fn cmd_status(server: &str) {
    match get_json(server, "/params") {
        Ok(p) => {
            println!("✅  Server: {server}");
            println!("   f0method:      {}", p["f0method"]);
            println!("   f0up_key:      {}", p["f0up_key"]);
            println!("   index_rate:    {}", p["index_rate"]);
        }
        Err(e) => {
            println!("❌  Server unreachable: {e}");
            std::process::exit(1);
        }
    }

    if let Ok(m) = get_json(server, "/models") {
        let models: Vec<String> = serde_json::from_value(m["models"].clone()).unwrap_or_default();
        let ready: Vec<String> =
            serde_json::from_value(m["onnx_ready"].clone()).unwrap_or_default();
        println!("   Models: {models:?}");
        println!("   ONNX ready: {ready:?}");
    }
}

fn cmd_analyse(file: &PathBuf, vs: Option<&PathBuf>, show_timeline: bool) {
    use foni_analyse::{
        analyse, compute_gap, decode_wav, format_gap_table, spectral_timeline, TargetTensor,
    };

    let bytes = std::fs::read(file).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    let wav = decode_wav(&bytes).unwrap_or_else(|e| {
        eprintln!("WAV decode: {e}");
        std::process::exit(1);
    });
    let analysis = analyse(&wav.samples, wav.sample_rate);

    if let Some(ref_path) = vs {
        let ref_bytes = std::fs::read(ref_path).expect("cannot read reference");
        let ref_wav = decode_wav(&ref_bytes).expect("cannot decode reference");
        let ref_an = analyse(&ref_wav.samples, ref_wav.sample_rate);
        let tensor = TargetTensor::from_analysis(&ref_an, ref_path.to_str().unwrap_or("?"));
        let gap = compute_gap(file.to_str().unwrap_or("?"), &analysis, &tensor);
        println!("{}", format_gap_table(&gap));

        if show_timeline {
            let tl = spectral_timeline::compare(
                &ref_wav.samples,
                &wav.samples,
                ref_wav.sample_rate,
                &ref_an.f0_contour,
                &analysis.f0_contour,
                &ref_an.energy_envelope,
                &analysis.energy_envelope,
            );
            println!("\nSpectral gap (LSD): {:.1} dB", tl.spectral_gap);
            println!("Pitch shape match:  {:.3}", tl.pitch_match);
            println!("Energy shape match: {:.3}", tl.energy_match);
            println!("\nWorst frames:");
            for (idx, gap_db) in &tl.worst_frames {
                let time_s = *idx as f32 * tl.frame_step_secs;
                println!("  {:.2}s  gap={:.1} dB", time_s, gap_db);
            }
            let spark = spectral_timeline::sparkline(&tl.spectral_gap_per_frame, 5);
            println!("\nPer-frame distance:\n  {spark}");
        }
    } else {
        println!("Duration:  {:.2}s", analysis.temporal.duration_secs);
        println!("RMS:       {:.1} dBFS", analysis.loudness.rms_db);
        println!("Crest:     {:.1} dB", analysis.loudness.crest_factor);
        println!("Brightness: {:.0} Hz", analysis.spectral.brightness_hz);
        println!(
            "Pauses:    {} × {:.0}ms",
            analysis.temporal.pause_count,
            analysis.temporal.mean_pause_duration * 1000.0
        );
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

/// Aggregate acoustic fingerprint across every WAV in a directory.
/// Runs in a single process — no subprocess overhead per file.
fn cmd_compare(
    server: &str,
    studio: &PathBuf,
    out_dir: &PathBuf,
    max_dur: f32,
    model: &str,
    skip_transcribe: bool,
) {
    use foni_analyse::{analyse_fast, decode_wav, transcribe};
    use rayon::prelude::*;
    use std::sync::Mutex;

    std::fs::create_dir_all(out_dir).expect("cannot create out_dir");

    // 1. Collect studio WAVs filtered by duration.
    let mut files: Vec<PathBuf> = std::fs::read_dir(studio)
        .unwrap_or_else(|e| {
            eprintln!("cannot read studio dir: {e}");
            std::process::exit(1);
        })
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("wav"))
        .filter(|p| {
            std::fs::read(p)
                .ok()
                .and_then(|b| decode_wav(&b).ok())
                .map(|w| w.samples.len() as f32 / w.sample_rate as f32 <= max_dur)
                .unwrap_or(false)
        })
        .collect();
    files.sort();

    if files.is_empty() {
        eprintln!("No WAVs ≤ {max_dur}s in {}", studio.display());
        return;
    }
    println!("\n∷ Compare: {} studio files ≤ {max_dur}s", files.len());

    // 2. Transcribe (or load cached .txt) and synthesize.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let client = foni_client::FoniClient::new(server);

    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("  [{bar:40}] {pos}/{len}  {msg}")
            .unwrap(),
    );

    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new(); // (studio, synthetic)
    let mut skipped = 0usize;

    for path in &files {
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        pb.set_message(stem.clone());

        let txt_path = out_dir.join(format!("{stem}.txt"));
        let synth_path = out_dir.join(format!("{stem}.wav"));

        // --- Transcribe ---
        let text = if skip_transcribe && txt_path.exists() {
            std::fs::read_to_string(&txt_path)
                .unwrap_or_default()
                .trim()
                .to_string()
        } else {
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    pb.println(format!("  skip {stem}: {e}"));
                    skipped += 1;
                    pb.inc(1);
                    continue;
                }
            };
            match transcribe(&bytes, "ru") {
                Some(t) => {
                    std::fs::write(&txt_path, &t).ok();
                    t
                }
                None => {
                    pb.println(format!("  skip {stem}: whisper failed"));
                    skipped += 1;
                    pb.inc(1);
                    continue;
                }
            }
        };

        if text.is_empty() {
            skipped += 1;
            pb.inc(1);
            continue;
        }

        // --- Synthesize ---
        if !synth_path.exists() || !skip_transcribe {
            let req = foni_client::SynthRequest {
                text: text.clone(),
                model: Some(model.to_string()),
                prosody: false,
                dsp: false, // raw RVC — same conditions as studio (no post-processing)
                ..foni_client::SynthRequest::new(&text)
            };
            match rt.block_on(client.synthesize(&req)) {
                Ok(wav) => {
                    std::fs::write(&synth_path, wav.as_bytes()).ok();
                }
                Err(e) => {
                    pb.println(format!("  skip {stem}: synth {e}"));
                    skipped += 1;
                    pb.inc(1);
                    continue;
                }
            }
        }

        pairs.push((path.clone(), synth_path));
        pb.inc(1);
    }
    pb.finish_and_clear();
    eprintln!("  Pairs: {}  Skipped: {skipped}", pairs.len());

    if pairs.is_empty() {
        return;
    }

    // 3. Analyse both sets in parallel.
    struct Row {
        rms: f64,
        crest: f64,
        centroid: f64,
        f0: f64,
        f0_std: f64,
        naturalness: Option<f32>,
    }
    let studio_rows = Mutex::new(Vec::<Row>::new());
    let synth_rows = Mutex::new(Vec::<Row>::new());

    pairs.par_iter().for_each(|(s_path, y_path)| {
        let analyse = |p: &PathBuf| -> Option<(Row, Vec<f32>, u32)> {
            let bytes = std::fs::read(p).ok()?;
            let wav = decode_wav(&bytes).ok()?;
            let a = analyse_fast(&wav.samples, wav.sample_rate);
            let (f0, f0s, _) = foni_analyse::fast_f0_stats(&wav.samples, wav.sample_rate);
            let row = Row {
                rms: a.loudness.rms_db as f64,
                crest: a.loudness.crest_factor as f64,
                centroid: a.spectral.brightness_hz as f64,
                f0: f0 as f64,
                f0_std: f0s as f64,
                naturalness: None,
            };
            Some((row, wav.samples, wav.sample_rate))
        };
        if let (Some((mut sr, _, _)), Some((sy, _, _))) = (analyse(s_path), analyse(y_path)) {
            let score = foni_analyse::naturalness::score(
                s_path.to_str().unwrap_or(""),
                y_path.to_str().unwrap_or(""),
            );
            sr.naturalness = score;
            studio_rows.lock().unwrap().push(sr);
            synth_rows.lock().unwrap().push(sy);
        }
    });

    let sr = studio_rows.into_inner().unwrap();
    let sy = synth_rows.into_inner().unwrap();
    let n = sr.len().min(sy.len()) as f64;
    if n == 0.0 {
        eprintln!("No analysed pairs.");
        return;
    }

    let mean = |rows: &[Row], f: fn(&Row) -> f64| rows.iter().map(|r| f(r)).sum::<f64>() / n;

    let metrics: &[(&str, fn(&Row) -> f64, &str)] = &[
        ("Pitch Hz", |r| r.f0, "bass-baritone 80–130 Hz"),
        ("Pitch variation Hz", |r| r.f0_std, "pitch variation"),
        (
            "Brightness Hz",
            |r| r.centroid,
            "bass<2400  baritone 2400–2700",
        ),
        ("RMS dBFS", |r| r.rms, "studio −≠13–−15 dBFS"),
        ("Crest factor dB", |r| r.crest, "speech 12–16 dB"),
    ];

    {
        use owo_colors::OwoColorize;
        use tabled::{settings::Style, Table, Tabled};
        #[derive(Tabled)]
        struct CompareRow {
            #[tabled(rename = "Metric")]
            metric: String,
            #[tabled(rename = "Studio")]
            studio: String,
            #[tabled(rename = "Synthetic")]
            synthetic: String,
            #[tabled(rename = "Gap%")]
            gap: String,
            #[tabled(rename = "Verdict")]
            verdict: String,
        }
        let mut rows = Vec::new();
        for (label, f, _target) in metrics {
            let sv = mean(&sr, *f);
            let yv = mean(&sy, *f);
            let gap = if sv.abs() > 0.01 {
                ((yv - sv).abs() / sv.abs() * 100.0)
            } else {
                0.0
            };
            let verdict = if gap < 10.0 {
                "close".green().to_string()
            } else if gap < 25.0 {
                "near".yellow().to_string()
            } else if gap < 50.0 {
                "far".red().to_string()
            } else {
                "very far".red().bold().to_string()
            };
            rows.push(CompareRow {
                metric: label.to_string(),
                studio: format!("{sv:.1}"),
                synthetic: format!("{yv:.1}"),
                gap: format!("{gap:.1}%"),
                verdict,
            });
        }
        let naturalness_scores: Vec<f32> = sr.iter().filter_map(|r| r.naturalness).collect();
        if !naturalness_scores.is_empty() {
            let mean_nat = naturalness_scores.iter().sum::<f32>() / naturalness_scores.len() as f32;
            let verdict = if mean_nat > 4.0 {
                "good".green().to_string()
            } else if mean_nat > 3.0 {
                "ok".yellow().to_string()
            } else {
                "poor".red().to_string()
            };
            rows.push(CompareRow {
                metric: "Naturalness (1-5)".into(),
                studio: "5.0".into(),
                synthetic: format!("{mean_nat:.2}"),
                gap: String::new(),
                verdict,
            });
        }
        println!("\n  Studio vs Synthetic ({} matched pairs)", n as usize);
        println!("{}", Table::new(&rows).with(Style::rounded()));
    }
    eprintln!("  Synthetic WAVs: {}/", out_dir.display());

    eprintln!("  Synthetic WAVs: {}/", out_dir.display());
}

fn cmd_tune(
    server: &str,
    text: &str,
    presets_path: &PathBuf,
    reference: Option<&std::path::Path>,
    model: &str,
) {
    use std::io::{BufRead, Write};

    let maquettes = load_maquettes(Some(presets_path));
    if maquettes.is_empty() {
        eprintln!("No presets in {}", presets_path.display());
        return;
    }

    println!("\n▶  Tune — {} presets", maquettes.len());
    println!("   Phrase: «{text}»");
    if let Some(r) = reference {
        println!("   Reference: {}", r.display());
    }
    println!("   Controls: 1–5 rate  n next  r replay  q quit\n");

    // Synthesize RVC base once, apply each preset via /process.
    eprint!("  Synthesizing RVC base… ");
    std::io::stdout().flush().ok();
    let base: Vec<u8> =
        match synth_request(server, text, model, "ru", 150, false, serde_json::json!({})) {
            Ok(b) => {
                eprintln!("ok ({} kB)", b.len() / 1024);
                b
            }
            Err(e) => {
                eprintln!("\n  ❌ {e}");
                return;
            }
        };

    let mut results: Vec<(String, u8, String)> = Vec::new(); // (name, rating, note)
    let stdin = std::io::stdin();
    let mut i = 0usize;

    while i < maquettes.len() {
        let m = &maquettes[i];
        let wav_path =
            std::env::temp_dir().join(format!("fonictl_tune_{}.wav", m.name.replace(' ', "_")));

        // Render preset.
        if !wav_path.exists() {
            eprint!("  Rendering «{}»… ", m.name);
            std::io::stdout().flush().ok();
            match process_request(server, &base, m.opts.clone()) {
                Ok(wav) => {
                    std::fs::write(&wav_path, &wav).unwrap();
                    eprintln!("ok");
                }
                Err(e) => {
                    println!("\n  ❌ {e}");
                    i += 1;
                    continue;
                }
            }
        }

        println!(
            "\n[{}/{}] «{}»  —  {}",
            i + 1,
            maquettes.len(),
            m.name,
            m.describe()
        );

        // Play reference first if provided.
        if let Some(r) = reference {
            println!("  ▶ reference");
            play_wav(r);
        }

        println!("  ▶ synthetic");
        play_wav(&wav_path);

        // Ask for rating.
        loop {
            print!("  Rating 1–5 (n=next  r=replay  q=quit)  > ");
            std::io::stdout().flush().ok();
            let mut line = String::new();
            stdin.lock().read_line(&mut line).ok();
            let line = line.trim();

            match line {
                "q" | "quit" => {
                    print_tune_results(&results);
                    return;
                }
                "n" | "next" | "" => {
                    i += 1;
                    break;
                }
                "r" | "replay" => {
                    if let Some(r) = reference {
                        play_wav(r);
                    }
                    play_wav(&wav_path);
                }
                s if s.len() == 1
                    && s.chars()
                        .next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false) =>
                {
                    let score: u8 = s.parse().unwrap_or(0);
                    if (1..=5).contains(&score) {
                        print!("  Note (Enter to skip): ");
                        std::io::stdout().flush().ok();
                        let mut note = String::new();
                        stdin.lock().read_line(&mut note).ok();
                        let note = note.trim().to_string();
                        results.push((m.name.clone(), score, note));
                        println!("  ★{}/5 saved", score);
                        i += 1;
                        break;
                    }
                }
                _ => println!("  ? type 1–5, r, n, or q"),
            }
        }
    }

    print_tune_results(&results);
}

fn print_tune_results(results: &[(String, u8, String)]) {
    if results.is_empty() {
        return;
    }
    let mut sorted = results.to_vec();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n╬══ Results ══════════════════════════════════════════");
    for (name, score, note) in &sorted {
        let stars = format!(
            "{}{}",
            "★".repeat(*score as usize),
            "☆".repeat(5 - *score as usize)
        );
        let note_str = if note.is_empty() {
            String::new()
        } else {
            format!(" — {note}")
        };
        println!("  {stars}  {name}{note_str}");
    }
    println!("╚════════════════════════════════════════════");
}

fn cmd_tune_auto(
    server: &str,
    phrase: &str,
    presets_path: &PathBuf,
    model: &str,
    n_iter: usize,
    vs: Option<&std::path::Path>,
    reference: Option<&std::path::Path>,
) {
    use foni_analyse::{analyse, compute_gap, decode_wav, format_gap_table, TargetTensor};
    use std::io::Write;

    // Gap score weights — brightness is the biggest problem, then loudness and voice presence
    const W_BRIGHTNESS: f32 = 0.35;
    const W_LOUDNESS: f32 = 0.25;
    const W_PRESENCE: f32 = 0.15;
    const W_DARKNESS: f32 = 0.15;
    const W_BASS: f32 = 0.10;

    // Load reference for gap computation
    let ref_path = vs.or(reference);
    let tensor = ref_path.and_then(|p| {
        let bytes = std::fs::read(p).ok()?;
        let wav = decode_wav(&bytes).ok()?;
        let analysis = analyse(&wav.samples, wav.sample_rate);
        Some(TargetTensor::from_analysis(
            &analysis,
            &p.display().to_string(),
        ))
    });

    let tensor = match tensor {
        Some(t) => t,
        None => {
            eprintln!("  ✗ --vs <reference.wav> required for auto-tuning");
            return;
        }
    };

    // Synthesize the base audio once (RVC only, no DSP)
    eprint!("  Synthesizing base (RVC only)… ");
    std::io::stdout().flush().ok();
    let base = match synth_request(
        server,
        phrase,
        model,
        "ru",
        150,
        false,
        serde_json::json!({}),
    ) {
        Ok(b) => {
            eprintln!("ok ({} kB)", b.len() / 1024);
            b
        }
        Err(e) => {
            eprintln!("\n  ✗ {e}");
            return;
        }
    };

    #[derive(Clone)]
    struct Knobs {
        tilt_low_db: f32,
        tilt_high_db: f32,
        rms_lufs: f32,
        compression: f32,
        presence_db: f32,
    }

    impl Knobs {
        fn to_opts(&self) -> serde_json::Value {
            serde_json::json!({
                "tiltLowDb": self.tilt_low_db,
                "tiltHighDb": self.tilt_high_db,
                "rmsTargetLufs": self.rms_lufs,
                "compressionRatio": self.compression,
                "presenceDb": self.presence_db,
                "vibratoDepth": 0.0,
            })
        }
    }

    let gap_score = |knobs: &Knobs| -> Option<f32> {
        let wav = process_request(server, &base, knobs.to_opts()).ok()?;
        let tmp = std::env::temp_dir().join("fonictl_auto_tune.wav");
        std::fs::write(&tmp, &wav).ok()?;
        let decoded = decode_wav(&wav).ok()?;
        let analysis = analyse(&decoded.samples, decoded.sample_rate);
        let gap = compute_gap("auto", &analysis, &tensor);

        // Weighted gap score — pull from named metrics
        let metric = |name: &str| {
            gap.rows
                .iter()
                .find(|r| r.metric == name)
                .map(|r| r.gap_pct)
                .unwrap_or(50.0)
        };
        let score = metric("Brightness") * W_BRIGHTNESS
            + metric("Loudness") * W_LOUDNESS
            + metric("Voice presence") * W_PRESENCE
            + metric("Vocal darkness") * W_DARKNESS
            + metric("Bass balance") * W_BASS;
        Some(score)
    };

    // Start from a neutral preset (or first from file)
    let maquettes = load_maquettes(Some(presets_path));
    let start_opts = maquettes
        .first()
        .map(|m| m.opts.clone())
        .unwrap_or_default();
    let mut best = Knobs {
        tilt_low_db: start_opts
            .get("tiltLowDb")
            .and_then(|v| v.as_f64())
            .unwrap_or(8.0) as f32,
        tilt_high_db: start_opts
            .get("tiltHighDb")
            .and_then(|v| v.as_f64())
            .unwrap_or(-6.0) as f32,
        rms_lufs: start_opts
            .get("rmsTargetLufs")
            .and_then(|v| v.as_f64())
            .unwrap_or(-16.0) as f32,
        compression: start_opts
            .get("compressionRatio")
            .and_then(|v| v.as_f64())
            .unwrap_or(4.0) as f32,
        presence_db: start_opts
            .get("presenceDb")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32,
    };

    let mut best_score = gap_score(&best).unwrap_or(100.0);

    eprintln!("\n  Starting gap score: {best_score:.1}%");
    eprintln!("  Running {n_iter} iterations of coordinate descent…");
    println!();

    let steps: &[(&str, &dyn Fn(&mut Knobs, f32), f32, f32, f32)] = &[
        ("tiltLowDb", &|k, d| k.tilt_low_db += d, 1.0, 0.0, 20.0),
        ("tiltHighDb", &|k, d| k.tilt_high_db += d, 1.0, -20.0, 0.0),
        ("rmsTargetLufs", &|k, d| k.rms_lufs += d, 0.5, -22.0, -8.0),
        (
            "compressionRatio",
            &|k, d| k.compression += d,
            0.5,
            1.0,
            10.0,
        ),
        ("presenceDb", &|k, d| k.presence_db += d, 0.5, -6.0, 6.0),
    ];

    let mut history: Vec<(Knobs, f32)> = vec![(best.clone(), best_score)];

    for iter in 0..n_iter {
        let mut changed = false;
        let mut best_knob = String::new();

        for (name, apply, step, lo, hi) in steps {
            for &delta in &[*step, -*step] {
                let mut candidate = best.clone();
                apply(&mut candidate, delta);
                // Clamp to bounds
                candidate.tilt_low_db = candidate.tilt_low_db.clamp(0.0, 20.0);
                candidate.tilt_high_db = candidate.tilt_high_db.clamp(-20.0, 0.0);
                candidate.rms_lufs = candidate.rms_lufs.clamp(-22.0, -8.0);
                candidate.compression = candidate.compression.clamp(1.0, 10.0);
                candidate.presence_db = candidate.presence_db.clamp(-6.0, 6.0);
                // Enforce bounds per knob
                let clamped = match *name {
                    "tiltLowDb" => candidate.tilt_low_db >= *lo && candidate.tilt_low_db <= *hi,
                    "tiltHighDb" => candidate.tilt_high_db >= *lo && candidate.tilt_high_db <= *hi,
                    "rmsTargetLufs" => candidate.rms_lufs >= *lo && candidate.rms_lufs <= *hi,
                    "compressionRatio" => {
                        candidate.compression >= *lo && candidate.compression <= *hi
                    }
                    "presenceDb" => candidate.presence_db >= *lo && candidate.presence_db <= *hi,
                    _ => true,
                };
                if !clamped {
                    continue;
                }

                if let Some(score) = gap_score(&candidate) {
                    if score < best_score {
                        best_score = score;
                        best = candidate;
                        best_knob = format!("{name}={delta:+.1}");
                        changed = true;
                    }
                }
            }
        }

        history.push((best.clone(), best_score));
        let mark = if changed {
            format!("← {best_knob}")
        } else {
            "(no improvement)".to_string()
        };
        println!(
            "  [{:>2}/{}]  gap {best_score:5.1}%  {mark}",
            iter + 1,
            n_iter
        );

        if !changed && iter > 3 {
            eprintln!("  Converged early at iteration {}.", iter + 1);
            break;
        }
    }

    // Save top-3 unique configs to presets file
    history.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    history.dedup_by(|a, b| (a.1 - b.1).abs() < 0.1);
    let top3: Vec<_> = history.into_iter().take(3).collect();

    let mut maquettes = load_maquettes(Some(presets_path));
    for (i, (knobs, score)) in top3.iter().enumerate() {
        let name = format!("auto-{} ({:.1}%)", i + 1, score);
        let opts = knobs.to_opts();
        if let Some(existing) = maquettes.iter_mut().find(|m| m.name.starts_with("auto-")) {
            existing.name = name;
            existing.opts = opts;
        } else {
            maquettes.push(Maquette { name, opts });
        }
    }

    let json = serde_json::to_string_pretty(&maquettes).unwrap_or_default();
    if std::fs::write(presets_path, &json).is_ok() {
        println!(
            "\n  Top-3 saved to {}  →  run `just tune` to listen",
            presets_path.display()
        );
    }

    eprintln!("\n  Best knobs:");
    eprintln!("    tiltLowDb:        {:.1}", top3[0].0.tilt_low_db);
    eprintln!("    tiltHighDb:       {:.1}", top3[0].0.tilt_high_db);
    eprintln!("    rmsTargetLufs:    {:.1}", top3[0].0.rms_lufs);
    eprintln!("    compressionRatio: {:.1}", top3[0].0.compression);
    eprintln!("    presenceDb:       {:.1}", top3[0].0.presence_db);
    eprintln!("    Final gap score:  {:.1}%", top3[0].1);
}

fn cmd_diff(server: &str, knob: &str, value: f32, phrase: &str, ref_path: &PathBuf, model: &str) {
    use foni_analyse::{analyse, compute_gap, decode_wav, spectral_timeline, TargetTensor};
    use std::io::Write;

    let ref_bytes = std::fs::read(ref_path).expect("cannot read reference WAV");
    let ref_wav = decode_wav(&ref_bytes).expect("reference WAV");
    let ref_an = analyse(&ref_wav.samples, ref_wav.sample_rate);

    eprint!("  Synthesizing base (RVC only)\u{2026} ");
    std::io::stdout().flush().ok();
    let base_wav = match synth_request(
        server,
        phrase,
        model,
        "ru",
        135,
        false,
        serde_json::json!({}),
    ) {
        Ok(b) => {
            eprintln!("ok");
            b
        }
        Err(e) => {
            eprintln!("\n  \u{2717} {e}");
            return;
        }
    };

    // Before: neutral DSP
    let neutral = serde_json::json!({});
    let before_wav = process_request(server, &base_wav, neutral).expect("neutral DSP");
    let before_decoded = decode_wav(&before_wav).expect("before WAV");
    let before_an = analyse(&before_decoded.samples, before_decoded.sample_rate);
    let before_gap = compute_gap(
        "before",
        &before_an,
        &TargetTensor::from_analysis(&ref_an, "ref"),
    );
    let before_tl = spectral_timeline::compare(
        &ref_wav.samples,
        &before_decoded.samples,
        ref_wav.sample_rate,
        &ref_an.f0_contour,
        &before_an.f0_contour,
        &ref_an.energy_envelope,
        &before_an.energy_envelope,
    );

    // After: with the knob changed
    let after_opts = serde_json::json!({ knob: value });
    let after_wav = process_request(server, &base_wav, after_opts).expect("modified DSP");
    let after_decoded = decode_wav(&after_wav).expect("after WAV");
    let after_an = analyse(&after_decoded.samples, after_decoded.sample_rate);
    let after_gap = compute_gap(
        "after",
        &after_an,
        &TargetTensor::from_analysis(&ref_an, "ref"),
    );
    let after_tl = spectral_timeline::compare(
        &ref_wav.samples,
        &after_decoded.samples,
        ref_wav.sample_rate,
        &ref_an.f0_contour,
        &after_an.f0_contour,
        &ref_an.energy_envelope,
        &after_an.energy_envelope,
    );

    // Report
    let gap_delta = after_gap.mean_gap_pct - before_gap.mean_gap_pct;
    let lsd_delta = after_tl.spectral_gap - before_tl.spectral_gap;
    let gap_arrow = if gap_delta < -1.0 {
        "\u{2193} improved"
    } else if gap_delta > 1.0 {
        "\u{2191} worse"
    } else {
        "\u{2194} same"
    };
    let lsd_arrow = if lsd_delta < -0.5 {
        "\u{2193} improved"
    } else if lsd_delta > 0.5 {
        "\u{2191} worse"
    } else {
        "\u{2194} same"
    };

    use owo_colors::OwoColorize;
    use tabled::{settings::Style, Table, Tabled};

    #[derive(Tabled)]
    struct DiffRow {
        #[tabled(rename = "Metric")]
        metric: String,
        #[tabled(rename = "Before")]
        before: String,
        #[tabled(rename = "After")]
        after: String,
        #[tabled(rename = "Delta")]
        delta: String,
    }

    let arrow = |d: f32, threshold: f32| -> String {
        if d < -threshold {
            format!("{:+.1} \u{2193}", d).green().to_string()
        } else if d > threshold {
            format!("{:+.1} \u{2191}", d).red().to_string()
        } else {
            format!("{:+.1} =", d).dimmed().to_string()
        }
    };

    let mut rows = vec![
        DiffRow {
            metric: "Mean gap".into(),
            before: format!("{:.1}%", before_gap.mean_gap_pct),
            after: format!("{:.1}%", after_gap.mean_gap_pct),
            delta: arrow(gap_delta, 1.0),
        },
        DiffRow {
            metric: "Spectral gap (LSD)".into(),
            before: format!("{:.1} dB", before_tl.spectral_gap),
            after: format!("{:.1} dB", after_tl.spectral_gap),
            delta: arrow(lsd_delta, 0.5),
        },
        DiffRow {
            metric: "Pitch match".into(),
            before: format!("{:.3}", before_tl.pitch_match),
            after: format!("{:.3}", after_tl.pitch_match),
            delta: String::new(),
        },
        DiffRow {
            metric: "Energy match".into(),
            before: format!("{:.3}", before_tl.energy_match),
            after: format!("{:.3}", after_tl.energy_match),
            delta: String::new(),
        },
    ];

    for name in [
        "Loudness",
        "Brightness",
        "Bass balance",
        "Vocal darkness",
        "Breathiness",
    ] {
        let b = before_gap.rows.iter().find(|r| r.metric == name);
        let a = after_gap.rows.iter().find(|r| r.metric == name);
        if let (Some(b), Some(a)) = (b, a) {
            let d = a.gap_pct - b.gap_pct;
            rows.push(DiffRow {
                metric: name.to_string(),
                before: format!("{:.1}%", b.gap_pct),
                after: format!("{:.1}%", a.gap_pct),
                delta: arrow(d, 1.0),
            });
        }
    }

    println!("\n  {} = {}", knob.bold(), value);
    println!("{}", Table::new(&rows).with(Style::rounded()));

    println!(
        "\n  Before: {}",
        spectral_timeline::sparkline(&before_tl.spectral_gap_per_frame, 5)
    );
    println!(
        "  After:  {}",
        spectral_timeline::sparkline(&after_tl.spectral_gap_per_frame, 5)
    );
}

fn cmd_calibrate(server: &str, phrase: &str, ref_path: &PathBuf, model: &str) {
    use foni_analyse::{analyse_fast, decode_wav};
    use std::io::Write;

    let ref_bytes = std::fs::read(ref_path).expect("cannot read reference WAV");
    let ref_wav = decode_wav(&ref_bytes).expect("reference WAV");
    let ref_a = analyse_fast(&ref_wav.samples, ref_wav.sample_rate);

    eprint!("  Synthesizing base (RVC, no DSP)\u{2026} ");
    std::io::stdout().flush().ok();
    let base = match synth_request(
        server,
        phrase,
        model,
        "ru",
        150,
        false,
        serde_json::json!({}),
    ) {
        Ok(b) => {
            eprintln!("ok ({} kB)", b.len() / 1024);
            b
        }
        Err(e) => {
            eprintln!("\n  \u{2717} {e}");
            return;
        }
    };

    struct Knob {
        name: &'static str,
        key: &'static str,
        step: f32,
    }
    let knobs = [
        Knob {
            name: "tiltLowDb",
            key: "tiltLowDb",
            step: 6.0,
        },
        Knob {
            name: "tiltHighDb",
            key: "tiltHighDb",
            step: -6.0,
        },
        Knob {
            name: "rmsTargetLufs",
            key: "rmsTargetLufs",
            step: 4.0,
        },
        Knob {
            name: "presenceDb",
            key: "presenceDb",
            step: 4.0,
        },
        Knob {
            name: "compressionRatio",
            key: "compressionRatio",
            step: 2.0,
        },
        Knob {
            name: "deHarshDb",
            key: "deHarshDb",
            step: -6.0,
        },
    ];

    struct Metric {
        name: &'static str,
        extract: fn(&foni_analyse::AnalysisResult) -> f32,
    }
    let metrics = [
        Metric {
            name: "brightness_hz",
            extract: |a| a.spectral.brightness_hz,
        },
        Metric {
            name: "loudness_db",
            extract: |a| a.loudness.rms_db,
        },
        Metric {
            name: "bass_balance_db",
            extract: |a| a.spectral.bass_balance_db,
        },
        Metric {
            name: "vocal_darkness_db_oct",
            extract: |a| a.spectral.vocal_darkness_db_oct,
        },
        Metric {
            name: "breathiness_db",
            extract: |a| a.voice.breathiness_db,
        },
    ];

    // Measure baseline (all knobs at zero / neutral)
    let neutral = serde_json::json!({
        "tiltLowDb": 0, "tiltHighDb": 0, "rmsTargetLufs": -16,
        "compressionRatio": 1, "presenceDb": 0, "deEssDb": 0, "vibratoDepth": 0
    });
    let base_wav = process_request(server, &base, neutral.clone()).expect("baseline DSP");
    let base_decoded = decode_wav(&base_wav).expect("baseline WAV");
    let base_a = analyse_fast(&base_decoded.samples, base_decoded.sample_rate);
    let base_vals: Vec<f32> = metrics.iter().map(|m| (m.extract)(&base_a)).collect();

    println!(
        "\n  Sweeping {} knobs \u{00d7} {} metrics\u{2026}\n",
        knobs.len(),
        metrics.len()
    );

    // Sensitivity table
    let mut matrix: Vec<Vec<f32>> = Vec::new();

    {
        use tabled::{settings::Style, Table, Tabled};
        #[derive(Tabled)]
        struct CalRow {
            #[tabled(rename = "Knob")]
            knob: String,
            #[tabled(rename = "Brightness")]
            brightness: String,
            #[tabled(rename = "Loudness")]
            loudness: String,
            #[tabled(rename = "Bass")]
            bass: String,
            #[tabled(rename = "Darkness")]
            darkness: String,
            #[tabled(rename = "Breathiness")]
            breathiness: String,
        }

        let mut rows = vec![CalRow {
            knob: "(baseline)".into(),
            brightness: format!("{:.2}", base_vals[0]),
            loudness: format!("{:.2}", base_vals[1]),
            bass: format!("{:.2}", base_vals[2]),
            darkness: format!("{:.2}", base_vals[3]),
            breathiness: format!("{:.2}", base_vals[4]),
        }];

        for knob in &knobs {
            let mut perturbed = neutral.clone();
            perturbed[knob.key] = serde_json::json!(knob.step);
            if knob.key == "rmsTargetLufs" {
                perturbed[knob.key] = serde_json::json!(-16.0 + knob.step);
            }

            let wav = process_request(server, &base, perturbed).expect("perturbed DSP");
            let decoded = decode_wav(&wav).expect("perturbed WAV");
            let a = analyse_fast(&decoded.samples, decoded.sample_rate);
            let vals: Vec<f32> = metrics.iter().map(|m| (m.extract)(&a)).collect();
            let sens: Vec<f32> = vals
                .iter()
                .zip(&base_vals)
                .map(|(v, b)| (v - b) / knob.step.abs())
                .collect();

            rows.push(CalRow {
                knob: format!("{}={:+.0}", knob.name, knob.step),
                brightness: format!("{:.3}", sens[0]),
                loudness: format!("{:.3}", sens[1]),
                bass: format!("{:.3}", sens[2]),
                darkness: format!("{:.3}", sens[3]),
                breathiness: format!("{:.3}", sens[4]),
            });
            matrix.push(sens);
        }

        println!("{}", Table::new(&rows).with(Style::rounded()));
    }

    // Print as Rust const for controller.rs
    println!("\n  \u{2500}\u{2500} Rust const for dsp/controller.rs \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\n");
    eprintln!("  // Rows: tiltLowDb, tiltHighDb, rmsTargetLufs, presenceDb, compressionRatio");
    eprintln!("  // Cols: brightness_hz, loudness_db, bass_balance_db, vocal_darkness_db_oct, breathiness_db");
    println!(
        "  const SENSITIVITY: [[f32; {}]; {}] = [",
        metrics.len(),
        knobs.len()
    );
    for (i, row) in matrix.iter().enumerate() {
        println!("      {:?}, // {}", row, knobs[i].name);
    }
    println!("  ];");

    // Target values from reference
    println!("\n  // Target values from reference WAV");
    let ref_fast = analyse_fast(&ref_wav.samples, ref_wav.sample_rate);
    let targets: Vec<f32> = metrics.iter().map(|m| (m.extract)(&ref_fast)).collect();
    println!("  const TARGET: [f32; {}] = {:?};", metrics.len(), targets);
}

fn cmd_corpus(dir: &PathBuf, vs: Option<&PathBuf>) {
    use foni_analyse::{
        analyse, analyse_fast, compute_gap, decode_wav, fast_f0_stats, format_gap_table,
        TargetTensor,
    };
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| {
            eprintln!("cannot read dir: {e}");
            std::process::exit(1);
        })
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("wav"))
        .collect();
    files.sort();

    if files.is_empty() {
        eprintln!("No WAV files in {}", dir.display());
        return;
    }

    let t0 = std::time::Instant::now();
    println!(
        "  Analysing {} files in parallel (fast F0 / McLeod, no pyin)…",
        files.len()
    );

    // Parallel accumulation — each file decoded and analysed independently.
    #[derive(Default)]
    struct Row {
        rms: f64,
        crest: f64,
        centroid: f64,
        f0: f64,
        f0_std: f64,
        voiced: f64,
    }
    let acc = Mutex::new(Vec::<Row>::new());
    let errors = AtomicU64::new(0);

    files.par_iter().for_each(|path| {
        match std::fs::read(path).and_then(|b| Ok(b)) {
            Ok(bytes) => match decode_wav(&bytes) {
                Ok(wav) => {
                    // Fast path: loudness + spectral + temporal (cheap) + McLeod F0.
                    // analyse_fast() skips pyin (1400 ms/file) and voice metrics.
                    let r = analyse_fast(&wav.samples, wav.sample_rate);
                    let (f0, f0_std, vr) = fast_f0_stats(&wav.samples, wav.sample_rate);
                    let row = Row {
                        rms: r.loudness.rms_db as f64,
                        crest: r.loudness.crest_factor as f64,
                        centroid: r.spectral.brightness_hz as f64,
                        f0: f0 as f64,
                        f0_std: f0_std as f64,
                        voiced: vr as f64,
                    };
                    acc.lock().unwrap().push(row);
                }
                Err(e) => {
                    eprintln!("  skip {}: {e}", path.display());
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            },
            Err(e) => {
                eprintln!("  skip {}: {e}", path.display());
                errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    let rows = acc.into_inner().unwrap();
    let n = rows.len();
    if n == 0 {
        eprintln!("All files failed.");
        return;
    }

    let mean = |f: fn(&Row) -> f64| rows.iter().map(|r| f(r)).sum::<f64>() / n as f64;
    let rms = mean(|r| r.rms);
    let crest = mean(|r| r.crest);
    let centroid = mean(|r| r.centroid);
    let f0 = mean(|r| r.f0);
    let f0_std = mean(|r| r.f0_std);
    let voiced = mean(|r| r.voiced);
    let elapsed = t0.elapsed().as_millis();
    let errs = errors.load(Ordering::Relaxed);

    eprintln!("  Done in {elapsed} ms  ({} files, {} skipped)\n", n, errs);

    // ── Sidorovich acoustic identity (bass-baritone deep Russian voice) ────────
    //
    // Parameters from literature (Kob et al. 2022, PMC9605961; Sundberg 1987;
    // SwiftF0 benchmark 2025):
    //
    //   F0 (speaking)  Bass:      75–100 Hz   Baritone: 96–130 Hz
    //   Spectral cent. Bass:    <2400 Hz      Baritone: 2400–2700 Hz
    //   FHE (1–5 kHz)  Bass:  2384±164 Hz     Baritone: 2454±206 Hz
    //   Crest factor   Conversational speech: 12–16 dB
    //   Voiced ratio   Clean studio speech:    60–85 %
    {
        use tabled::{settings::Style, Table, Tabled};
        #[derive(Tabled)]
        struct CorpusRow {
            #[tabled(rename = "Metric")]
            metric: &'static str,
            #[tabled(rename = "Value")]
            value: String,
            #[tabled(rename = "Target")]
            target: &'static str,
        }
        let rows = vec![
            CorpusRow {
                metric: "Pitch",
                value: format!("{f0:.1} Hz"),
                target: "bass-baritone: 80-130 Hz",
            },
            CorpusRow {
                metric: "Pitch variation",
                value: format!("{f0_std:.1} Hz"),
                target: "higher = more expressive",
            },
            CorpusRow {
                metric: "Brightness",
                value: format!("{centroid:.0} Hz"),
                target: "bass<2400, baritone 2400-2700",
            },
            CorpusRow {
                metric: "RMS level",
                value: format!("{rms:.1} dBFS"),
                target: "studio: -13 to -15 dBFS",
            },
            CorpusRow {
                metric: "Crest factor",
                value: format!("{crest:.1} dB"),
                target: "speech: 12-16 dB",
            },
            CorpusRow {
                metric: "Voiced ratio",
                value: format!("{:.1}%", voiced * 100.0),
                target: "studio: 60-85%",
            },
        ];
        eprintln!("\n  Sidorovich corpus fingerprint ({n} files, {elapsed} ms, {errs} skipped)");
        println!("{}", Table::new(&rows).with(Style::rounded()));
    }

    if let Some(ref_path) = vs {
        let ref_bytes = std::fs::read(ref_path).expect("cannot read reference");
        let ref_wav = decode_wav(&ref_bytes).expect("cannot decode reference");
        let ref_an = analyse(&ref_wav.samples, ref_wav.sample_rate);
        let tensor = TargetTensor::from_analysis(&ref_an, ref_path.to_str().unwrap_or("?"));
        let med_bytes = std::fs::read(&files[files.len() / 2]).expect("cannot read median file");
        let med_wav = decode_wav(&med_bytes).expect("cannot decode median");
        let med_an = analyse(&med_wav.samples, med_wav.sample_rate);
        let gap = compute_gap("corpus-median", &med_an, &tensor);
        println!("\n(median file vs reference WAV)");
        println!("{}", format_gap_table(&gap));
    }
}

fn main() {
    let cli = Cli::parse();
    let server = cli.server.trim_end_matches('/');

    match cli.cmd {
        Cmd::Synth {
            text,
            play,
            out,
            model,
            voice,
            speed,
            no_dsp,
            rms_target_lufs,
            compression_ratio,
            tilt_low_db,
            tilt_high_db,
            vibrato_freq,
            vibrato_depth,
            presence_db,
            de_ess_db,
        } => {
            cmd_synth(
                server,
                &text,
                &model,
                &voice,
                speed,
                !no_dsp,
                out.as_ref(),
                play,
                rms_target_lufs,
                compression_ratio,
                tilt_low_db,
                tilt_high_db,
                vibrato_freq,
                vibrato_depth,
                presence_db,
                de_ess_db,
            );
        }
        Cmd::Studio { text, model, from } => {
            cmd_studio(server, &text, &model, from.as_deref());
        }
        Cmd::Samples { out_dir, model } => {
            cmd_samples(server, &out_dir, &model);
        }
        Cmd::Status => {
            cmd_status(server);
        }
        Cmd::Process {
            file,
            out,
            opts,
            vs,
        } => {
            cmd_process(server, &file, out.as_ref(), &opts, vs.as_ref());
        }
        Cmd::Play { file } => {
            play_wav(&file);
        }
        Cmd::Analyse { file, vs, timeline } => {
            cmd_analyse(&file, vs.as_ref(), timeline);
        }
        Cmd::Compare {
            studio,
            out_dir,
            max_dur,
            model,
            skip_transcribe,
        } => {
            cmd_compare(server, &studio, &out_dir, max_dur, &model, skip_transcribe);
        }
        Cmd::Tune {
            text,
            presets,
            reference,
            model,
            auto,
            vs,
        } => {
            if let Some(n_iter) = auto {
                cmd_tune_auto(
                    server,
                    &text,
                    &presets,
                    &model,
                    n_iter,
                    vs.as_deref(),
                    reference.as_deref(),
                );
            } else {
                cmd_tune(server, &text, &presets, reference.as_deref(), &model);
            }
        }
        Cmd::Corpus { dir, vs } => {
            cmd_corpus(&dir, vs.as_ref());
        }
        Cmd::Calibrate { text, vs, model } => {
            cmd_calibrate(server, &text, &vs, &model);
        }
        Cmd::Diff {
            knob,
            value,
            text,
            vs,
            model,
        } => {
            cmd_diff(server, &knob, value, &text, &vs, &model);
        }
        Cmd::Mix {
            text,
            model,
            from,
            reference,
        } => {
            cmd_mix(server, &text, &model, from.as_deref(), reference.as_deref());
        }
        Cmd::Listen {
            text,
            model,
            dsp,
            diagnose,
            vs,
        } => {
            if diagnose {
                cmd_diagnose(server, &text, &model);
            } else {
                cmd_listen(server, &text, &model, dsp, vs);
            }
        }
    }
}
