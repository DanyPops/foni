use super::cmd_common::{play_wav, process_request, synth_request};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

pub fn cmd_analyse(file: &PathBuf, vs: Option<&PathBuf>, show_timeline: bool) {
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

            // Tempo comparison
            let tc =
                foni_analyse::tempo::compare(&ref_wav.samples, &wav.samples, ref_wav.sample_rate);
            println!("\nTempo: overall {:.0}% speed", tc.overall_ratio * 100.0);
            if !tc.rushed.is_empty() {
                println!("  Rushed segments:");
                for r in &tc.rushed {
                    println!(
                        "    {:.2}s  ref={:.0}ms  syn={:.0}ms  ({:.0}%)",
                        r.ref_start_s,
                        r.ref_duration_s * 1000.0,
                        r.syn_duration_s * 1000.0,
                        r.ratio * 100.0
                    );
                }
            }
            if !tc.drawn_out.is_empty() {
                println!("  Drawn out segments:");
                for d in &tc.drawn_out {
                    println!(
                        "    {:.2}s  ref={:.0}ms  syn={:.0}ms  ({:.0}%)",
                        d.ref_start_s,
                        d.ref_duration_s * 1000.0,
                        d.syn_duration_s * 1000.0,
                        d.ratio * 100.0
                    );
                }
            }
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

pub fn cmd_compare(
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

pub fn cmd_sweep(
    server: &str,
    knob: &str,
    values_csv: &str,
    phrase: &str,
    ref_path: &PathBuf,
    model: &str,
) {
    use foni_analyse::{analyse, compute_gap, decode_wav, spectral_timeline, TargetTensor};
    use owo_colors::OwoColorize;
    use std::io::Write;
    use tabled::{settings::Style, Table, Tabled};

    let values: Vec<f32> = values_csv
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if values.is_empty() {
        eprintln!("No valid values in '{values_csv}'");
        return;
    }

    let ref_bytes = std::fs::read(ref_path).expect("cannot read reference");
    let ref_wav = decode_wav(&ref_bytes).expect("reference WAV");
    let ref_an = analyse(&ref_wav.samples, ref_wav.sample_rate);
    let tensor = TargetTensor::from_analysis(&ref_an, "ref");

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

    #[derive(Tabled)]
    struct SweepRow {
        #[tabled(rename = "Value")]
        value: String,
        #[tabled(rename = "Mean gap")]
        mean_gap: String,
        #[tabled(rename = "LSD")]
        lsd: String,
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

    // Baseline (neutral DSP)
    let neutral_wav = process_request(server, &base_wav, serde_json::json!({})).expect("neutral");
    let neutral_dec = decode_wav(&neutral_wav).expect("neutral WAV");
    let neutral_an = analyse(&neutral_dec.samples, neutral_dec.sample_rate);
    let neutral_gap = compute_gap("neutral", &neutral_an, &tensor);
    let neutral_tl = spectral_timeline::compare(
        &ref_wav.samples,
        &neutral_dec.samples,
        ref_wav.sample_rate,
        &ref_an.f0_contour,
        &neutral_an.f0_contour,
        &ref_an.energy_envelope,
        &neutral_an.energy_envelope,
    );

    let metric_gap = |gap: &foni_analyse::GapResult, name: &str| -> f32 {
        gap.rows
            .iter()
            .find(|r| r.metric == name)
            .map(|r| r.gap_pct)
            .unwrap_or(0.0)
    };

    let mut rows = vec![SweepRow {
        value: "(neutral)".dimmed().to_string(),
        mean_gap: format!("{:.1}%", neutral_gap.mean_gap_pct),
        lsd: format!("{:.1}", neutral_tl.spectral_gap),
        brightness: format!("{:.1}%", metric_gap(&neutral_gap, "Brightness")),
        loudness: format!("{:.1}%", metric_gap(&neutral_gap, "Loudness")),
        bass: format!("{:.1}%", metric_gap(&neutral_gap, "Bass balance")),
        darkness: format!("{:.1}%", metric_gap(&neutral_gap, "Vocal darkness")),
        breathiness: format!("{:.1}%", metric_gap(&neutral_gap, "Breathiness")),
    }];

    let mut best_val = f32::NAN;
    let mut best_gap = neutral_gap.mean_gap_pct;

    for &val in &values {
        let opts = serde_json::json!({ knob: val });
        let wav = process_request(server, &base_wav, opts).expect("sweep DSP");
        let dec = decode_wav(&wav).expect("sweep WAV");
        let an = analyse(&dec.samples, dec.sample_rate);
        let gap = compute_gap("sweep", &an, &tensor);
        let tl = spectral_timeline::compare(
            &ref_wav.samples,
            &dec.samples,
            ref_wav.sample_rate,
            &ref_an.f0_contour,
            &an.f0_contour,
            &ref_an.energy_envelope,
            &an.energy_envelope,
        );

        if gap.mean_gap_pct < best_gap {
            best_gap = gap.mean_gap_pct;
            best_val = val;
        }

        rows.push(SweepRow {
            value: format!("{val}"),
            mean_gap: format!("{:.1}%", gap.mean_gap_pct),
            lsd: format!("{:.1}", tl.spectral_gap),
            brightness: format!("{:.1}%", metric_gap(&gap, "Brightness")),
            loudness: format!("{:.1}%", metric_gap(&gap, "Loudness")),
            bass: format!("{:.1}%", metric_gap(&gap, "Bass balance")),
            darkness: format!("{:.1}%", metric_gap(&gap, "Vocal darkness")),
            breathiness: format!("{:.1}%", metric_gap(&gap, "Breathiness")),
        });
    }

    eprintln!("\n  Sweep: {knob}");
    println!("{}", Table::new(&rows).with(Style::rounded()));
    if best_val.is_finite() {
        eprintln!(
            "\n  Best: {} = {}  (mean gap {:.1}%)",
            knob.bold(),
            best_val,
            best_gap
        );
    }
}

pub fn cmd_diff(
    server: &str,
    knob: &str,
    value: f32,
    phrase: &str,
    ref_path: &PathBuf,
    model: &str,
) {
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

pub fn cmd_calibrate(server: &str, phrase: &str, ref_path: &PathBuf, model: &str) {
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
