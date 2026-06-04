use super::cmd_common::{
    load_maquettes, play_wav, print_tune_results, process_request, synth_request, Maquette,
};
use dialoguer::{theme::ColorfulTheme, Input, Select};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

pub fn cmd_tune(
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

pub fn cmd_tune_auto(
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

pub fn cmd_test_policy(script: &PathBuf, brightness: f32, loudness: f32, bass: f32, darkness: f32) {
    use owo_colors::OwoColorize;
    use tabled::{settings::Style, Table, Tabled};

    let engine = match foni_synth::quality::dsp::policy::PolicyEngine::load(script) {
        Some(e) => e,
        None => {
            eprintln!("  Failed to load script: {}", script.display());
            return;
        }
    };

    let cfg = foni_synth::quality::dsp::controller::ControllerConfig::default();
    let base = foni_synth::quality::dsp::SmoothingOptions::default();

    let mut analysis = foni_analyse::AnalysisResult {
        temporal: Default::default(),
        loudness: Default::default(),
        spectral: Default::default(),
        pitch: Default::default(),
        voice: Default::default(),
        f0_contour: vec![],
        energy_envelope: vec![],
    };
    analysis.spectral.brightness_hz = brightness;
    analysis.loudness.rms_db = loudness;
    analysis.spectral.bass_balance_db = bass;
    analysis.spectral.vocal_darkness_db_oct = darkness;

    match engine.evaluate(&analysis, &base, &cfg) {
        Some((opts, snap)) => {
            #[derive(Tabled)]
            struct Row {
                #[tabled(rename = "Knob")]
                knob: &'static str,
                #[tabled(rename = "Base")]
                base: String,
                #[tabled(rename = "Corrected")]
                corrected: String,
                #[tabled(rename = "Delta")]
                delta: String,
            }
            let rows = vec![
                Row {
                    knob: "tiltHighDb",
                    base: format!("{:.1}", base.tilt_high_db),
                    corrected: format!("{:.1}", opts.tilt_high_db),
                    delta: format!("{:+.1}", snap.correction_tilt_high_db),
                },
                Row {
                    knob: "tiltLowDb",
                    base: format!("{:.1}", base.tilt_low_db),
                    corrected: format!("{:.1}", opts.tilt_low_db),
                    delta: format!("{:+.1}", snap.correction_tilt_low_db),
                },
                Row {
                    knob: "rmsTargetLufs",
                    base: format!("{:.1}", base.rms_target_lufs),
                    corrected: format!("{:.1}", opts.rms_target_lufs),
                    delta: format!("{:+.1}", snap.correction_rms_lufs),
                },
                Row {
                    knob: "presenceDb",
                    base: format!("{:.1}", base.presence_db),
                    corrected: format!("{:.1}", opts.presence_db),
                    delta: format!("{:+.1}", snap.correction_presence_db),
                },
                Row {
                    knob: "deHarshDb",
                    base: format!("{:.1}", base.de_harsh_db),
                    corrected: format!("{:.1}", opts.de_harsh_db),
                    delta: format!("{:+.1}", snap.correction_de_harsh_db),
                },
            ];
            eprintln!("  Input: brightness={brightness} loudness={loudness} bass={bass} darkness={darkness}");
            println!("{}", Table::new(&rows).with(Style::rounded()));
        }
        None => {
            eprintln!("  Script evaluation failed. Check for runtime errors.");
        }
    }
}
