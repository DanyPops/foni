// fonictl — foni-synth command-line factory.
// Commands: synth | knobs | samples | status | play | analyse

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

    /// Interactive knob tuning loop
    Knobs {
        /// Phrase to synthesize during tuning
        #[arg(default_value = "Подойди-ка, надо тебе ситуацию прояснить.")]
        text: String,
        #[arg(short, long, default_value = "bandit")]
        model: String,
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

fn cmd_knobs(server: &str, text: &str, model: &str) {
    let theme = ColorfulTheme::default();

    // Knob definitions: (display name, json key, default, step)
    struct Knob {
        name: &'static str,
        key: &'static str,
        val: f32,
        step: f32,
        min: f32,
        max: f32,
    }
    let mut knobs = vec![
        Knob {
            name: "rmsTargetLufs     (loudness)",
            key: "rmsTargetLufs",
            val: -8.0,
            step: 1.0,
            min: -30.0,
            max: 0.0,
        },
        Knob {
            name: "compressionRatio  (dynamics)",
            key: "compressionRatio",
            val: 4.0,
            step: 0.5,
            min: 1.0,
            max: 20.0,
        },
        Knob {
            name: "compressionMakeupDb (gain)",
            key: "compressionMakeupDb",
            val: 5.0,
            step: 0.5,
            min: 0.0,
            max: 20.0,
        },
        Knob {
            name: "tiltLowDb         (warmth)",
            key: "tiltLowDb",
            val: 10.0,
            step: 1.0,
            min: -20.0,
            max: 20.0,
        },
        Knob {
            name: "tiltHighDb        (air cut)",
            key: "tiltHighDb",
            val: -8.0,
            step: 1.0,
            min: -20.0,
            max: 20.0,
        },
        Knob {
            name: "presenceDb        (clarity)",
            key: "presenceDb",
            val: 0.0,
            step: 0.5,
            min: -10.0,
            max: 10.0,
        },
        Knob {
            name: "deEssDb           (de-ess)",
            key: "deEssDb",
            val: 4.0,
            step: 0.5,
            min: 0.0,
            max: 15.0,
        },
        Knob {
            name: "vibratoFreq       (Hz)",
            key: "vibratoFreq",
            val: 6.0,
            step: 0.5,
            min: 0.0,
            max: 15.0,
        },
        Knob {
            name: "vibratoDepth      (amount)",
            key: "vibratoDepth",
            val: 0.003,
            step: 0.001,
            min: 0.0,
            max: 0.02,
        },
    ];

    println!("🎛  Knob tuning mode");
    println!("    Phrase: «{}»", text);
    println!("    Model:  {model}");
    println!("    Commands: [s]ynth  [+/-] adjust  [q]uit  [p]rint opts\n");

    loop {
        // Print current values
        println!("  Current knobs:");
        for (i, k) in knobs.iter().enumerate() {
            println!("  {:2}. {:40}  {:>8.3}", i + 1, k.name, k.val);
        }
        println!();

        let actions = vec![
            "▶  Synthesize and play",
            "✏  Adjust a knob",
            "↺  Reset to defaults",
            "💾 Save opts to JSON",
            "🖨  Print curl command",
            "✗  Quit",
        ];

        let choice = Select::with_theme(&theme)
            .with_prompt("Action")
            .items(&actions)
            .default(0)
            .interact()
            .unwrap_or(5);

        match choice {
            0 => {
                // Synthesize and play
                let mut opts = serde_json::json!({});
                for k in &knobs {
                    opts[k.key] = k.val.into();
                }

                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner} {msg}")
                        .unwrap(),
                );
                pb.set_message("Synthesizing...");
                pb.enable_steady_tick(std::time::Duration::from_millis(80));

                match synth_request(server, text, model, "ru", 150, true, opts) {
                    Ok(bytes) => {
                        pb.finish_and_clear();
                        let path = save_and_maybe_play(&bytes, None, true);
                        println!("  ✅  {} ({} kB)\n", path.display(), bytes.len() / 1024);
                    }
                    Err(e) => {
                        pb.finish_and_clear();
                        println!("  ❌  {e}\n");
                    }
                }
            }
            1 => {
                // Adjust a knob
                let names: Vec<_> = knobs.iter().map(|k| k.name).collect();
                let idx = Select::with_theme(&theme)
                    .with_prompt("Which knob")
                    .items(&names)
                    .interact()
                    .unwrap_or(0);

                let k = &mut knobs[idx];
                let ops = vec![
                    format!(
                        "+ {:.3} (→ {:.3})",
                        k.step,
                        (k.val + k.step).clamp(k.min, k.max)
                    ),
                    format!(
                        "- {:.3} (→ {:.3})",
                        k.step,
                        (k.val - k.step).clamp(k.min, k.max)
                    ),
                    format!("Enter value (current: {:.3})", k.val),
                ];

                let op = Select::with_theme(&theme)
                    .with_prompt("Adjust")
                    .items(&ops)
                    .interact()
                    .unwrap_or(0);

                match op {
                    0 => k.val = (k.val + k.step).clamp(k.min, k.max),
                    1 => k.val = (k.val - k.step).clamp(k.min, k.max),
                    _ => {
                        let v: f32 = Input::with_theme(&theme)
                            .with_prompt(format!("{} value", k.name))
                            .default(k.val)
                            .interact_text()
                            .unwrap_or(k.val);
                        k.val = v.clamp(k.min, k.max);
                    }
                }
                println!("  → {} = {:.3}\n", k.name, k.val);
            }
            2 => {
                // Reset defaults
                for k in &mut knobs {
                    k.val = match k.key {
                        "rmsTargetLufs" => -8.0,
                        "compressionRatio" => 4.0,
                        "compressionMakeupDb" => 5.0,
                        "tiltLowDb" => 10.0,
                        "tiltHighDb" => -8.0,
                        "vibratoFreq" => 6.0,
                        "vibratoDepth" => 0.003,
                        _ => 0.0,
                    };
                }
                println!("  ↺  Reset to defaults\n");
            }
            3 => {
                // Print JSON
                let mut opts = serde_json::json!({});
                for k in &knobs {
                    opts[k.key] = k.val.into();
                }
                println!("{}\n", serde_json::to_string_pretty(&opts).unwrap());
            }
            4 => {
                // Print curl command
                let mut opts = serde_json::json!({});
                for k in &knobs {
                    opts[k.key] = k.val.into();
                }
                let body = serde_json::json!({
                    "text": text, "model": model, "voice": "ru",
                    "speed": 150, "dsp": true, "opts": opts
                });
                println!("curl -s -X POST {server}/synthesize \\\n  -H 'Content-Type: application/json' \\\n  -d '{}' \\\n  -o out.wav\n",
                    serde_json::to_string(&body).unwrap());
            }
            _ => break,
        }
    }
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
        Cmd::Knobs { text, model } => {
            cmd_knobs(server, &text, &model);
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
    }
}
