// fonictl — foni-synth command-line factory.
// Commands: synth | studio | samples | listen | status | play | analyse

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
        #[arg(short, long, default_value = "bandit")]
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
        #[arg(short, long, default_value = "bandit")]
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
        #[arg(short, long, default_value = "bandit")]
        model: String,
    },

    /// Print server health and loaded model
    Status,

    /// Render pipeline stages or DSP variants and play interactively
    Listen {
        /// Phrase to synthesize
        #[arg(default_value = "Подойди-ка, надо тебе ситуацию прояснить.")]
        text: String,
        /// Model name
        #[arg(short, long, default_value = "bandit")]
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

    /// Play a WAV file via system player
    Play { file: PathBuf },

    /// Print acoustic metrics for a WAV file
    Analyse {
        file: PathBuf,
        /// Compare against reference WAV
        #[arg(long)]
        vs: Option<PathBuf>,
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

fn play_wav(path: &std::path::Path) {
    // Try aplay → paplay → afplay → mpv → ffplay in order
    for player in &["aplay", "paplay", "afplay", "mpv", "ffplay"] {
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
    eprintln!("⚠  No audio player found (tried aplay, paplay, afplay, mpv, ffplay)");
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

    println!("\n⚠  Diagnose — isolating noise sources");
    println!("   Phrase: «{text}»");
    println!("   Step 1: synthesizing RVC base (no DSP) …");

    // Synthesize RVC without DSP once — this is the base for all variants.
    let rvc_wav = match synth_request(server, text, model, "ru", 150, false, serde_json::json!({}))
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("  ❌ RVC synthesis failed: {e}");
            return;
        }
    };
    println!("   Base: {} kB", rvc_wav.len() / 1024);

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

fn cmd_analyse(file: &PathBuf, vs: Option<&PathBuf>) {
    use foni_analyse::{analyse, compute_gap, decode_wav, format_gap_table, TargetTensor};

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
    } else {
        println!("Duration:  {:.2}s", analysis.temporal.duration_secs);
        println!("RMS:       {:.1} dBFS", analysis.loudness.rms_db);
        println!("Crest:     {:.1} dB", analysis.loudness.crest_factor);
        println!("Centroid:  {:.0} Hz", analysis.spectral.centroid_hz);
        println!(
            "Pauses:    {} × {:.0}ms",
            analysis.temporal.pause_count,
            analysis.temporal.mean_pause_duration * 1000.0
        );
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

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
        Cmd::Play { file } => {
            play_wav(&file);
        }
        Cmd::Analyse { file, vs } => {
            cmd_analyse(&file, vs.as_ref());
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
