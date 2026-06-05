use super::cmd_common::{data_dir, synth_request};
use super::modal_cloud;
use std::path::PathBuf;
use tracing::{debug, error, info, warn};

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
    info!("\n  ▶  Fish Speech training [{mode}]");
    info!("  Model: {model}");
    info!("  Steps: {steps}");
    info!("  Cloud: Modal (L4 GPU)");

    if dry_run {
        info!("✓ Dry run — would spawn Modal function:");
        info!("  modal_cloud::spawn_training(\"{model}\", {steps})");
        info!("  Then poll/stream logs until completion.");
        return;
    }

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mut client = match modal_cloud::connect().await {
            Ok(c) => c,
            Err(e) => {
                error!("{e}");
                info!("  Run: modal token new");
                return;
            }
        };
        info!("✓ Modal connected");

        let call_id = match modal_cloud::spawn_training(&mut client, model, steps).await {
            Ok(id) => id,
            Err(e) => {
                error!("spawn failed: {e}");
                info!("  Run: modal deploy training/modal-train.py");
                return;
            }
        };
        info!("✓ Job spawned: {call_id}");

        if !follow {
            info!("\n  Job running in background.");
            info!("Check status:  fonictl train-status {call_id}");
            info!("Stream logs:   fonictl train-logs {call_id}");
            info!("Cancel:        fonictl train-cancel {call_id}");
            return;
        }

        info!("▶ Tailing logs...\n");
        match modal_cloud::tail_logs(&mut client, &call_id, 50).await {
            Ok(lines) => {
                for line in &lines {
                    eprint!("{line}");
                }
                // Check final status
                match modal_cloud::job_status(&mut client, &call_id).await {
                    Ok(modal_cloud::JobStatus::Success(r)) => info!("\n  ✓ Complete: {r}"),
                    Ok(modal_cloud::JobStatus::Running) => {
                        info!("\n  ⏳ Still running. Check: fonictl train-status {call_id}")
                    }
                    Ok(modal_cloud::JobStatus::Failed(r)) => info!("\n  ✗ Failed: {r}"),
                    Err(e) => info!("\n  ✗ {e}"),
                }
            }
            Err(e) => error!("{e}"),
        }
    });
}

pub fn cmd_train_status(call_id: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mut client = match modal_cloud::connect().await {
            Ok(c) => c,
            Err(e) => {
                error!("{e}");
                return;
            }
        };

        match modal_cloud::job_status(&mut client, call_id).await {
            Ok(modal_cloud::JobStatus::Success(result)) => println!("✓ Complete: {result}"),
            Ok(modal_cloud::JobStatus::Running) => println!("⏳ Running..."),
            Ok(modal_cloud::JobStatus::Failed(reason)) => info!("✗ Failed: {reason}"),
            Err(e) => info!("✗ {e}"),
        }
    });
}

pub fn cmd_train_logs(call_id: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mut client = match modal_cloud::connect().await {
            Ok(c) => c,
            Err(e) => {
                error!("{e}");
                return;
            }
        };

        match modal_cloud::tail_logs(&mut client, call_id, 10).await {
            Ok(lines) => {
                if lines.is_empty() {
                    info!("(no logs yet)");
                } else {
                    for line in &lines {
                        eprint!("{line}");
                    }
                }
            }
            Err(e) => info!("✗ {e}"),
        }
    });
}

pub fn cmd_train_cancel(call_id: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mut client = match modal_cloud::connect().await {
            Ok(c) => c,
            Err(e) => {
                error!("{e}");
                return;
            }
        };

        match modal_cloud::cancel_job(&mut client, call_id).await {
            Ok(()) => println!("✓ Job cancelled"),
            Err(e) => info!("✗ {e}"),
        }
    });
}

pub fn cmd_snapshot(server: &str, model: &str, ref_path: &PathBuf) -> Result<(), String> {
    use foni_analyse::{analyse, compute_gap, decode_wav, TargetTensor};
    use owo_colors::OwoColorize;

    info!("\n  ▶  Snapshot — saving baseline scores for {model}");

    let ref_bytes = std::fs::read(ref_path).map_err(|e| format!("{}: {e}", ref_path.display()))?;
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
                info!("✗ {e}");
                continue;
            }
        };
        let synth_wav = decode_wav(&wav).expect("decode synth WAV");
        let synth_analysis = analyse(&synth_wav.samples, synth_wav.sample_rate);
        let gap = compute_gap(phrase, &synth_analysis, &tensor);
        tracing::info!(
            "gap={:.1}%  «{}»",
            gap.mean_gap_pct,
            &phrase[..phrase.len().min(40)]
        );
        scores.push(gap.mean_gap_pct);
    }

    if scores.is_empty() {
        return Err("no scores".into());
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
    tracing::info!(
        "\n  ✓ Baseline saved: {} (mean gap {:.1}%)",
        path.display(),
        avg.bold()
    );
    Ok(())
}

pub fn cmd_compare_models(server: &str, model: &str, ref_path: &PathBuf) -> Result<(), String> {
    use foni_analyse::{analyse, compute_gap, decode_wav, TargetTensor};
    use owo_colors::OwoColorize;

    let baseline_path = data_dir().join("baselines").join(format!("{model}.json"));
    let baseline: serde_json::Value = match std::fs::read_to_string(&baseline_path) {
        Ok(s) => serde_json::from_str(&s).expect("parse baseline JSON"),
        Err(_) => {
            return Err(format!(
                "No baseline at {}. Run: fonictl snapshot {model} --vs {}",
                baseline_path.display(),
                ref_path.display()
            ));
        }
    };

    let old_avg = baseline["mean_gap_pct"].as_f64().unwrap_or(100.0) as f32;

    let ref_bytes = std::fs::read(ref_path).map_err(|e| format!("{}: {e}", ref_path.display()))?;
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
                info!("✗ {e}");
                continue;
            }
        };
        let synth_wav = decode_wav(&wav).expect("decode synth WAV");
        let synth_analysis = analyse(&synth_wav.samples, synth_wav.sample_rate);
        let gap = compute_gap(phrase, &synth_analysis, &tensor);
        tracing::info!("gap={:.1}%", gap.mean_gap_pct);
        scores.push(gap.mean_gap_pct);
    }

    if scores.is_empty() {
        return Err("no scores".into());
    }

    let new_avg: f32 = scores.iter().sum::<f32>() / scores.len() as f32;
    let delta = old_avg - new_avg;

    info!("\n  ═══ Comparison ═══");
    tracing::info!("    Baseline:  {:.1}%", old_avg);
    tracing::info!("    Current:   {:.1}%", new_avg);
    if delta > 0.0 {
        tracing::info!(
            "    Result:    {} ({:.1}% improvement)",
            "PASS ✓".green().bold(),
            delta
        );
    } else {
        tracing::info!(
            "    Result:    {} ({:.1}% regression)",
            "FAIL ✗".red().bold(),
            -delta
        );
    }
    Ok(())
}

pub fn cmd_tts_compare(phrase: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let client = reqwest::Client::new();
        let token = std::env::var("FONI_TTS_TOKEN").unwrap_or_default();
        let body = serde_json::json!({"text": phrase, "language": "ru", "token": token});

        info!("\n  ▶  Comparing TTS models in parallel");
        tracing::info!(
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
                tracing::info!("  ✓ Chatterbox:  {} bytes, {}ms", wav.len(), cb_ms);
                info!("  Playing...");
                super::cmd_common::play_wav(std::path::Path::new(path));
            }
            Ok(resp) => tracing::info!("  ✗ Chatterbox: HTTP {}", resp.status()),
            Err(e) => error!("Chatterbox: {e}"),
        }

        let fs_ms = t0.elapsed().as_millis();

        match fs_res {
            Ok(resp) if resp.status().is_success() => {
                let wav = resp.bytes().await.unwrap_or_default();
                let path = "/tmp/fonictl_compare_fish.wav";
                std::fs::write(path, &wav).ok();
                tracing::info!("  ✓ Fish S2-Pro: {} bytes, {}ms", wav.len(), fs_ms);
                info!("  Playing...");
                super::cmd_common::play_wav(std::path::Path::new(path));
            }
            Ok(resp) => {
                let body = resp.text().await.unwrap_or_default();
                error!("Fish S2-Pro: {body}");
            }
            Err(e) => error!("Fish S2-Pro: {e}"),
        }
    });
}

pub fn cmd_tts_bench(url: &str, phrase: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let client = reqwest::Client::new();
        let token = std::env::var("FONI_TTS_TOKEN").unwrap_or_default();
        let body = serde_json::json!({"text": phrase, "language": "ru", "token": token});

        info!("\n  ▶  TTS latency benchmark");
        info!("  Endpoint: {url}");
        tracing::info!(
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
                    tracing::info!(
                        "  [{i}] {label:4} {ms:>6}ms  {:.0}KB  {dur_secs:.1}s audio  RTF={rtf:.1}x",
                        bytes.len() as f64 / 1024.0
                    );
                    results.push(ms);

                    if i == 0 {
                        let path = "/tmp/fonictl_bench.wav";
                        std::fs::write(path, &bytes).ok();
                        info!("     Playing...");
                        super::cmd_common::play_wav(std::path::Path::new(path));
                    }
                }
                Ok(r) => tracing::info!("  [{i}] {label:4} HTTP {}", r.status()),
                Err(e) => info!("[{i}] {label:4} {e}"),
            }
        }

        if results.len() >= 2 {
            let warm_avg = results[1..].iter().sum::<u64>() / (results.len() - 1) as u64;
            tracing::info!("\n  Cold:     {}ms", results[0]);
            tracing::info!("  Warm avg: {}ms", warm_avg);
        }
    });
}
