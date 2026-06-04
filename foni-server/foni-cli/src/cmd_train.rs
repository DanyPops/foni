use super::cmd_common::{data_dir, synth_request};
use super::modal_cloud;
use std::path::PathBuf;

const SNAPSHOT_PHRASES: &[&str] = &[
    "Подойди-ка, надо тебе ситуацию прояснить.",
    "Здравствуй, сталкер. Чего тебе надо?",
    "Осторожно. Здесь аномалии, не зевай.",
    "Деплой прошёл успешно, коммиты запушены.",
    "Удачи, браток. На Зоне удача нужна.",
];

pub fn cmd_train(
    _server: &str,
    model: &str,
    _dataset: &PathBuf,
    _ref_path: &PathBuf,
    steps: u32,
    dry_run: bool,
    _ntfy_topic: &str,
    follow: bool,
) {
    use owo_colors::OwoColorize;

    let mode = if dry_run {
        "DRY RUN".yellow().bold().to_string()
    } else {
        "LIVE".green().bold().to_string()
    };
    eprintln!("\n  ▶  Fish Speech training [{mode}]");
    eprintln!("    Model: {model}");
    eprintln!("    Steps: {steps}");
    eprintln!("    Cloud: Modal (L4 GPU)");
    eprintln!();

    if dry_run {
        eprintln!("  ✓ Dry run — would spawn Modal function:");
        eprintln!("    modal_cloud::spawn_training(\"{model}\", {steps})");
        eprintln!("    Then poll/stream logs until completion.");
        return;
    }

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mut client = match modal_cloud::connect().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  ✗ {e}");
                eprintln!("    Run: modal token new");
                return;
            }
        };
        eprintln!("  ✓ Modal connected");

        let call_id = match modal_cloud::spawn_training(&mut client, model, steps).await {
            Ok(id) => id,
            Err(e) => {
                eprintln!("  ✗ spawn failed: {e}");
                eprintln!("    Run: modal deploy training/modal-train.py");
                return;
            }
        };
        eprintln!("  ✓ Job spawned: {call_id}");

        if !follow {
            eprintln!("\n  Job running in background.");
            eprintln!("  Check status:  fonictl train-status {call_id}");
            eprintln!("  Stream logs:   fonictl train-logs {call_id}");
            eprintln!("  Cancel:        fonictl train-cancel {call_id}");
            return;
        }

        eprintln!("  ▶ Streaming logs...\n");
        match modal_cloud::stream_logs(&mut client, &call_id).await {
            Ok(Some(result)) => {
                eprintln!("\n  ✓ Training complete: {result}");
            }
            Ok(None) => {
                eprintln!("\n  ⚠ Logs ended but no result yet");
                eprintln!("  Check: fonictl train-status {call_id}");
            }
            Err(e) => {
                eprintln!("\n  ✗ {e}");
            }
        }
    });
}

pub fn cmd_train_status(call_id: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mut client = match modal_cloud::connect().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  ✗ {e}");
                return;
            }
        };

        match modal_cloud::poll_result(&mut client, call_id).await {
            Ok(Some(result)) => println!("✓ Complete: {result}"),
            Ok(None) => println!("⏳ Still running..."),
            Err(e) => eprintln!("✗ {e}"),
        }
    });
}

pub fn cmd_train_logs(call_id: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mut client = match modal_cloud::connect().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  ✗ {e}");
                return;
            }
        };

        match modal_cloud::stream_logs(&mut client, call_id).await {
            Ok(Some(result)) => eprintln!("\n✓ Complete: {result}"),
            Ok(None) => eprintln!("\n⏳ Logs ended, job may still be running"),
            Err(e) => eprintln!("\n✗ {e}"),
        }
    });
}

pub fn cmd_train_cancel(call_id: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mut client = match modal_cloud::connect().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  ✗ {e}");
                return;
            }
        };

        match modal_cloud::cancel_job(&mut client, call_id).await {
            Ok(()) => println!("✓ Job cancelled"),
            Err(e) => eprintln!("✗ {e}"),
        }
    });
}

pub fn cmd_snapshot(server: &str, model: &str, ref_path: &PathBuf) {
    use foni_analyse::{analyse, compute_gap, decode_wav, TargetTensor};
    use owo_colors::OwoColorize;

    eprintln!("\n  ▶  Snapshot — saving baseline scores for {model}");

    let ref_bytes = std::fs::read(ref_path).unwrap_or_else(|e| {
        eprintln!("  ✗ {}: {e}", ref_path.display());
        std::process::exit(1);
    });
    let ref_wav = decode_wav(&ref_bytes).expect("decode reference WAV");
    let ref_analysis = analyse(&ref_wav.samples, ref_wav.sample_rate);
    let tensor = TargetTensor::from_analysis(&ref_analysis, ref_path.to_str().unwrap_or("ref"));

    let mut scores = Vec::new();
    for (i, phrase) in SNAPSHOT_PHRASES.iter().enumerate() {
        eprint!("  [{}/{}] ", i + 1, SNAPSHOT_PHRASES.len());
        let wav = match synth_request(
            server,
            phrase,
            model,
            "ru",
            150,
            true,
            serde_json::json!({}),
        ) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("✗ {e}");
                continue;
            }
        };
        let synth_wav = decode_wav(&wav).expect("decode synth WAV");
        let synth_analysis = analyse(&synth_wav.samples, synth_wav.sample_rate);
        let gap = compute_gap(phrase, &synth_analysis, &tensor);
        eprintln!(
            "gap={:.1}%  «{}»",
            gap.mean_gap_pct,
            &phrase[..phrase.len().min(40)]
        );
        scores.push(gap.mean_gap_pct);
    }

    if scores.is_empty() {
        eprintln!("  ✗ no scores");
        return;
    }

    let avg: f32 = scores.iter().sum::<f32>() / scores.len() as f32;
    let dir = data_dir().join("baselines");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(format!("{model}.json"));
    let snapshot = serde_json::json!({
        "model": model,
        "mean_gap_pct": avg,
        "scores": scores,
        "phrases": SNAPSHOT_PHRASES,
        "reference": ref_path.display().to_string(),
    });
    std::fs::write(&path, serde_json::to_string_pretty(&snapshot).unwrap()).ok();
    eprintln!(
        "\n  ✓ Baseline saved: {} (mean gap {:.1}%)",
        path.display(),
        avg.bold()
    );
}

pub fn cmd_compare_models(server: &str, model: &str, ref_path: &PathBuf) {
    use foni_analyse::{analyse, compute_gap, decode_wav, TargetTensor};
    use owo_colors::OwoColorize;

    let baseline_path = data_dir().join("baselines").join(format!("{model}.json"));
    let baseline: serde_json::Value = match std::fs::read_to_string(&baseline_path) {
        Ok(s) => serde_json::from_str(&s).expect("parse baseline JSON"),
        Err(_) => {
            eprintln!("  ✗ No baseline found at {}", baseline_path.display());
            eprintln!(
                "    Run: fonictl snapshot {model} --vs {}",
                ref_path.display()
            );
            return;
        }
    };

    let old_avg = baseline["mean_gap_pct"].as_f64().unwrap_or(100.0) as f32;

    let ref_bytes = std::fs::read(ref_path).unwrap_or_else(|e| {
        eprintln!("  ✗ {}: {e}", ref_path.display());
        std::process::exit(1);
    });
    let ref_wav = decode_wav(&ref_bytes).expect("decode ref WAV");
    let ref_analysis = analyse(&ref_wav.samples, ref_wav.sample_rate);
    let tensor = TargetTensor::from_analysis(&ref_analysis, ref_path.to_str().unwrap_or("ref"));

    let mut scores = Vec::new();
    for (i, phrase) in SNAPSHOT_PHRASES.iter().enumerate() {
        eprint!("  [{}/{}] ", i + 1, SNAPSHOT_PHRASES.len());
        let wav = match synth_request(
            server,
            phrase,
            model,
            "ru",
            150,
            true,
            serde_json::json!({}),
        ) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("✗ {e}");
                continue;
            }
        };
        let synth_wav = decode_wav(&wav).expect("decode synth WAV");
        let synth_analysis = analyse(&synth_wav.samples, synth_wav.sample_rate);
        let gap = compute_gap(phrase, &synth_analysis, &tensor);
        eprintln!("gap={:.1}%", gap.mean_gap_pct);
        scores.push(gap.mean_gap_pct);
    }

    if scores.is_empty() {
        eprintln!("  ✗ no scores");
        return;
    }

    let new_avg: f32 = scores.iter().sum::<f32>() / scores.len() as f32;
    let delta = old_avg - new_avg;

    eprintln!("\n  ═══ Comparison ═══");
    eprintln!("    Baseline:  {:.1}%", old_avg);
    eprintln!("    Current:   {:.1}%", new_avg);
    if delta > 0.0 {
        eprintln!(
            "    Result:    {} ({:.1}% improvement)",
            "PASS ✓".green().bold(),
            delta
        );
    } else {
        eprintln!(
            "    Result:    {} ({:.1}% regression)",
            "FAIL ✗".red().bold(),
            -delta
        );
    }
}
