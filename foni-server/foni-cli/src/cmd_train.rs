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

        eprintln!("  ▶ Tailing logs...\n");
        match modal_cloud::tail_logs(&mut client, &call_id, 50).await {
            Ok(lines) => {
                for line in &lines {
                    eprint!("{line}");
                }
                // Check final status
                match modal_cloud::job_status(&mut client, &call_id).await {
                    Ok(modal_cloud::JobStatus::Success(r)) => eprintln!("\n  ✓ Complete: {r}"),
                    Ok(modal_cloud::JobStatus::Running) => {
                        eprintln!("\n  ⏳ Still running. Check: fonictl train-status {call_id}")
                    }
                    Ok(modal_cloud::JobStatus::Failed(r)) => eprintln!("\n  ✗ Failed: {r}"),
                    Err(e) => eprintln!("\n  ✗ {e}"),
                }
            }
            Err(e) => eprintln!("  ✗ {e}"),
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

        match modal_cloud::job_status(&mut client, call_id).await {
            Ok(modal_cloud::JobStatus::Success(result)) => println!("✓ Complete: {result}"),
            Ok(modal_cloud::JobStatus::Running) => println!("⏳ Running..."),
            Ok(modal_cloud::JobStatus::Failed(reason)) => eprintln!("✗ Failed: {reason}"),
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

        match modal_cloud::tail_logs(&mut client, call_id, 10).await {
            Ok(lines) => {
                if lines.is_empty() {
                    eprintln!("(no logs yet)");
                } else {
                    for line in &lines {
                        eprint!("{line}");
                    }
                }
            }
            Err(e) => eprintln!("✗ {e}"),
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

pub fn cmd_tts_compare(phrase: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let client = reqwest::Client::new();
        let token = std::env::var("FONI_TTS_TOKEN").unwrap_or_default();
        let body = serde_json::json!({"text": phrase, "language": "ru", "token": token});

        eprintln!("\n  ▶  Comparing TTS models in parallel");
        eprintln!(
            "    Phrase: «{}»\n",
            phrase.chars().take(40).collect::<String>()
        );

        let t0 = std::time::Instant::now();

        let cb_fut = client
            .post("https://dpopsuev--chatterbox.modal.run")
            .json(&body)
            .timeout(std::time::Duration::from_secs(300))
            .send();
        let fs_fut = client
            .post("https://dpopsuev--fish.modal.run")
            .json(&body)
            .timeout(std::time::Duration::from_secs(300))
            .send();

        let (cb_res, fs_res) = tokio::join!(cb_fut, fs_fut);

        let cb_ms = t0.elapsed().as_millis();

        match cb_res {
            Ok(resp) if resp.status().is_success() => {
                let wav = resp.bytes().await.unwrap_or_default();
                let path = "/tmp/fonictl_compare_chatterbox.wav";
                std::fs::write(path, &wav).ok();
                eprintln!("  ✓ Chatterbox:  {} bytes, {}ms", wav.len(), cb_ms);
                eprintln!("    Playing...");
                super::cmd_common::play_wav(std::path::Path::new(path));
            }
            Ok(resp) => eprintln!("  ✗ Chatterbox: HTTP {}", resp.status()),
            Err(e) => eprintln!("  ✗ Chatterbox: {e}"),
        }

        let fs_ms = t0.elapsed().as_millis();

        match fs_res {
            Ok(resp) if resp.status().is_success() => {
                let wav = resp.bytes().await.unwrap_or_default();
                let path = "/tmp/fonictl_compare_fish.wav";
                std::fs::write(path, &wav).ok();
                eprintln!("  ✓ Fish S2-Pro: {} bytes, {}ms", wav.len(), fs_ms);
                eprintln!("    Playing...");
                super::cmd_common::play_wav(std::path::Path::new(path));
            }
            Ok(resp) => {
                let body = resp.text().await.unwrap_or_default();
                eprintln!("  ✗ Fish S2-Pro: {body}");
            }
            Err(e) => eprintln!("  ✗ Fish S2-Pro: {e}"),
        }
    });
}

pub fn cmd_tts_bench(url: &str, phrase: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let client = reqwest::Client::new();
        let token = std::env::var("FONI_TTS_TOKEN").unwrap_or_default();
        let body = serde_json::json!({"text": phrase, "language": "ru", "token": token});

        eprintln!("\n  ▶  TTS latency benchmark");
        eprintln!("    Endpoint: {url}");
        eprintln!(
            "    Phrase:   «{}»\n",
            phrase.chars().take(50).collect::<String>()
        );

        let mut results = Vec::new();
        for i in 0..3 {
            let label = if i == 0 { "cold" } else { "warm" };
            let t0 = std::time::Instant::now();
            let resp = client
                .post(url)
                .json(&body)
                .timeout(std::time::Duration::from_secs(300))
                .send()
                .await;
            let ms = t0.elapsed().as_millis() as u64;

            match resp {
                Ok(r) if r.status().is_success() => {
                    let bytes = r.bytes().await.unwrap_or_default();
                    let dur_secs = bytes.len() as f64 / (22050.0 * 2.0);
                    let rtf = ms as f64 / 1000.0 / dur_secs;
                    eprintln!(
                        "  [{i}] {label:4} {ms:>6}ms  {:.0}KB  {dur_secs:.1}s audio  RTF={rtf:.1}x",
                        bytes.len() as f64 / 1024.0
                    );
                    results.push(ms);

                    if i == 0 {
                        let path = "/tmp/fonictl_bench.wav";
                        std::fs::write(path, &bytes).ok();
                        eprintln!("       Playing...");
                        super::cmd_common::play_wav(std::path::Path::new(path));
                    }
                }
                Ok(r) => eprintln!("  [{i}] {label:4} HTTP {}", r.status()),
                Err(e) => eprintln!("  [{i}] {label:4} {e}"),
            }
        }

        if results.len() >= 2 {
            let warm_avg = results[1..].iter().sum::<u64>() / (results.len() - 1) as u64;
            eprintln!("\n  Cold:     {}ms", results[0]);
            eprintln!("  Warm avg: {}ms", warm_avg);
        }
    });
}
