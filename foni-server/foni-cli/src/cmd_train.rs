use super::cloud;
use super::cmd_common::{data_dir, synth_request};
use super::cost;
use std::path::PathBuf;

const SNAPSHOT_PHRASES: &[&str] = &[
    "Подойди-ка, надо тебе ситуацию прояснить.",
    "Здравствуй, сталкер. Чего тебе надо?",
    "Осторожно. Здесь аномалии, не зевай.",
    "Деплой прошёл успешно, коммиты запушены.",
    "Удачи, браток. На Зоне удача нужна.",
];
pub fn cmd_train(
    server: &str,
    model: &str,
    dataset: &PathBuf,
    ref_path: &PathBuf,
    epochs: u32,
    dry_run: bool,
    ntfy_topic: &str,
    follow: bool,
) {
    use super::cloud::CloudProvider;
    use owo_colors::OwoColorize;
    use std::io::Write;

    let mode = if dry_run {
        "DRY RUN".yellow().bold().to_string()
    } else {
        "LIVE".green().bold().to_string()
    };
    eprintln!("\n  \u{25b6}  Training pipeline [{mode}]");
    eprintln!("    Model:   {model}");
    eprintln!("    Dataset: {}", dataset.display());
    eprintln!("    Epochs:  {epochs}");
    eprintln!();

    // Build provider
    let endpoint_id =
        std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_else(|_| "foni-rvc-train".into());
    let api_key = match std::env::var("RUNPOD_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("  \u{2717} RUNPOD_API_KEY not set");
            return;
        }
    };
    let provider = cloud::RunPodProvider::new(&api_key);

    // Budget guard
    let balance_before = provider.balance().map(|b| b.balance).unwrap_or(0.0);
    if !dry_run && balance_before < 0.50 {
        eprintln!(
            "  \u{2717} Balance too low: ${:.2}. Need at least $0.50.",
            balance_before
        );
        return;
    }
    eprintln!("  Balance: ${:.2}", balance_before);

    let dataset_files = std::fs::read_dir(dataset)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "wav")
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    eprintln!(
        "  Dataset: {} WAV files in {}",
        dataset_files,
        dataset.display()
    );

    // Step 4: Create pod
    let started_at = chrono::Utc::now().to_rfc3339();
    let t_start = std::time::Instant::now();

    if dry_run {
        eprintln!("  [4/7] (dry-run) Would create pod");
        eprintln!(
            "  [5/7] (dry-run) Would upload {} files and train",
            dataset_files
        );
        eprintln!("  [6/7] (dry-run) Would download model");
        eprintln!("  [4/4] Comparing models\u{2026}");
        cmd_compare_models(server, model, ref_path);

        let receipt = cost::Receipt {
            timestamp: chrono::Utc::now().to_rfc3339(),
            model_name: model.into(),
            action: "dry-run".into(),
            gpu: "mock".into(),
            pod_id: "dry-run".into(),
            provider: "MockProvider".into(),
            started_at: started_at.clone(),
            finished_at: chrono::Utc::now().to_rfc3339(),
            duration_min: 0.0,
            cost_per_hr: 0.0,
            cost_usd: 0.0,
            balance_before,
            balance_after: balance_before,
            epochs,
            final_loss: 0.001,
            dataset_files,
            dataset_duration_min: dataset_files as f64 * 9.3 / 63.0,
            old_mean_gap: 39.5,
            new_mean_gap: 39.5,
            passed: false,
        };
        cost::print_receipt(&receipt);
        cost::save_receipt(receipt);
        return;
    }

    let gpu = std::env::var("FONI_GPU").unwrap_or_else(|_| "NVIDIA RTX A5000".into());
    let image = std::env::var("FONI_TRAIN_IMAGE")
        .unwrap_or_else(|_| "runpod/pytorch:1.0.2-cu1281-torch280-ubuntu2404".into());

    eprintln!("  [1/4] Creating pod ({gpu})\u{2026}");

    let template_id = std::env::var("FONI_TEMPLATE_ID").unwrap_or_else(|_| "zqsos1r03t".into());

    let gh_token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
    let pod_opts = cloud::CreatePodOpts {
        gpu_type_id: gpu.clone(),
        image: String::new(),
        volume_gb: 0,
        container_disk_gb: 20,
        name: "foni-train".into(),
        ports: String::new(),
        docker_args: String::new(),
        template_id: Some(template_id),
        env: {
            let mut env = vec![
                ("FONI_MODEL".into(), model.to_string()),
                ("FONI_EPOCHS".into(), epochs.to_string()),
                ("FONI_DATASET_URL".into(), std::env::var("FONI_DATASET_URL")
                    .unwrap_or_else(|_| "https://github.com/DanyPops/foni/releases/download/dataset-fish/foni-dataset-fish.tar.gz".into())),
                ("FONI_UPLOAD_TAG".into(), format!("model-{model}-fish")),
            ];
            if !gh_token.is_empty() {
                env.push(("GITHUB_TOKEN".into(), gh_token));
            }
            env
        },
    };
    let gpu_candidates = [
        "NVIDIA GeForce RTX 3090",
        "NVIDIA RTX A5000",
        "NVIDIA GeForce RTX 4090",
        "NVIDIA RTX A6000",
        "NVIDIA A40",
        "NVIDIA L4",
        "NVIDIA GeForce RTX 3090 Ti",
        "NVIDIA RTX PRO 6000 Blackwell Server Edition",
        "NVIDIA RTX PRO 6000 Blackwell Workstation Edition",
        "NVIDIA RTX PRO 4500 Blackwell",
    ];
    let pod = 'retry: loop {
        for candidate in std::iter::once(gpu.as_str()).chain(gpu_candidates.iter().copied()) {
            let mut opts = pod_opts.clone();
            opts.gpu_type_id = candidate.to_string();
            if let Ok(p) = provider.create_pod(opts) {
                eprintln!(
                    "    Pod: {} ({}), ${:.2}/hr",
                    p.id, p.gpu_name, p.cost_per_hr
                );
                break 'retry p;
            }
        }
        eprint!("\r    No GPUs available, retrying in 30s...");
        std::io::Write::flush(&mut std::io::stderr()).ok();
        std::thread::sleep(std::time::Duration::from_secs(30));
    };

    // Step 5: Training runs inside dockerArgs — poll pod status for EXITED
    eprintln!("  [2/4] Training (running in pod)\u{2026}");
    eprintln!("    Logs: https://console.runpod.io/pods");

    let mut final_loss = 0.001f64;
    let train_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1800);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
        match provider.get_pod(&pod.id) {
            Ok(p) => {
                let status = p["desiredStatus"].as_str().unwrap_or("?");
                if status == "EXITED" || status == "STOPPED" {
                    eprintln!("\n    Pod exited \u{2014} training done");
                    break;
                }
                eprint!("\r    {status}  ");
                std::io::Write::flush(&mut std::io::stderr()).ok();
            }
            Err(_) => {
                eprintln!("\n    Pod gone \u{2014} training may have completed");
                break;
            }
        }
        if std::time::Instant::now() > train_deadline {
            eprintln!("\n  \u{2717} Training timed out (30 min)");
            provider.terminate_pod(&pod.id).ok();
            return;
        }
    }

    // Step 6: Download model from GitHub release (uploaded by pod-train.py)
    eprintln!("  [3/4] Downloading model\u{2026}");
    let model_dir = format!("training/models/{model}");
    std::fs::create_dir_all(&model_dir).ok();
    let download_url = format!(
        "https://github.com/DanyPops/foni/releases/download/model-{model}-fish/{model}-fish.tar.gz"
    );
    let dl_status = std::process::Command::new("curl")
        .args([
            "-sL",
            "-o",
            &format!("{model_dir}/{model}.pth"),
            &download_url,
        ])
        .status();
    match dl_status {
        Ok(s) if s.success() => eprintln!("    Downloaded from {download_url}"),
        _ => eprintln!("  \u{26a0} Download failed"),
    }
    // Kill pod + compute cost
    let elapsed = t_start.elapsed();
    let duration_min = elapsed.as_secs_f64() / 60.0;
    let cost_usd = duration_min / 60.0 * pod.cost_per_hr;
    provider.terminate_pod(&pod.id).ok();
    eprintln!(
        "    Pod terminated. {:.1}min, ${:.4}",
        duration_min, cost_usd
    );

    // Step 7: Compare models
    eprintln!("  [4/4] Comparing models\u{2026}");
    cmd_compare_models(server, model, ref_path);

    let balance_after = provider
        .balance()
        .map(|b| b.balance)
        .unwrap_or(balance_before);

    let receipt = cost::Receipt {
        timestamp: chrono::Utc::now().to_rfc3339(),
        model_name: model.into(),
        action: "train".into(),
        gpu: gpu,
        pod_id: pod.id,
        provider: "RunPod".into(),
        started_at,
        finished_at: chrono::Utc::now().to_rfc3339(),
        duration_min,
        cost_per_hr: pod.cost_per_hr,
        cost_usd,
        balance_before,
        balance_after,
        epochs,
        final_loss,
        dataset_files,
        dataset_duration_min: dataset_files as f64 * 9.3 / 63.0,
        old_mean_gap: 39.5,
        new_mean_gap: 39.5,
        passed: false,
    };
    cost::print_receipt(&receipt);
    cost::save_receipt(receipt);
}

pub fn cmd_snapshot(server: &str, model: &str, ref_path: &PathBuf) {
    use foni_analyse::{analyse, compute_gap, decode_wav, spectral_timeline, TargetTensor};
    use std::io::Write;

    let ref_bytes = std::fs::read(ref_path).expect("cannot read reference");
    let ref_wav = decode_wav(&ref_bytes).expect("reference WAV");
    let ref_an = analyse(&ref_wav.samples, ref_wav.sample_rate);
    let tensor = TargetTensor::from_analysis(&ref_an, "ref");

    let mut results: Vec<serde_json::Value> = Vec::new();

    for (i, phrase) in SNAPSHOT_PHRASES.iter().enumerate() {
        eprint!(
            "  [{}/{}] {}\u{2026} ",
            i + 1,
            SNAPSHOT_PHRASES.len(),
            phrase.chars().take(25).collect::<String>()
        );
        std::io::stderr().flush().ok();
        let wav = match synth_request(
            server,
            phrase,
            model,
            "ru",
            135,
            false,
            serde_json::json!({}),
        ) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("skip: {e}");
                continue;
            }
        };
        let decoded = decode_wav(&wav).expect("synth WAV");
        let an = analyse(&decoded.samples, decoded.sample_rate);
        let gap = compute_gap(phrase, &an, &tensor);
        let tl = spectral_timeline::compare(
            &ref_wav.samples,
            &decoded.samples,
            ref_wav.sample_rate,
            &ref_an.f0_contour,
            &an.f0_contour,
            &ref_an.energy_envelope,
            &an.energy_envelope,
        );
        eprintln!(
            "gap {:.1}%  LSD {:.1} dB",
            gap.mean_gap_pct, tl.spectral_gap
        );
        results.push(serde_json::json!({
            "phrase": phrase,
            "mean_gap": gap.mean_gap_pct,
            "spectral_gap": tl.spectral_gap,
            "pitch_match": tl.pitch_match,
            "energy_match": tl.energy_match,
            "brightness": an.spectral.brightness_hz,
            "voice_presence": an.pitch.voice_presence,
            "worst_frame": tl.worst_frames.first().map(|f| f.1).unwrap_or(0.0),
        }));
    }

    let n = results.len() as f32;
    let avg = |key: &str| {
        results
            .iter()
            .map(|r| r[key].as_f64().unwrap_or(0.0) as f32)
            .sum::<f32>()
            / n
    };
    let snapshot = serde_json::json!({
        "model": model,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "phrases": results,
        "aggregate": {
            "mean_gap": avg("mean_gap"),
            "spectral_gap": avg("spectral_gap"),
            "pitch_match": avg("pitch_match"),
            "brightness": avg("brightness"),
            "voice_presence": avg("voice_presence"),
            "worst_frame": avg("worst_frame"),
        },
    });

    let dir = data_dir().join("baselines");
    std::fs::create_dir_all(&dir).expect("create baselines dir");
    let path = dir.join(format!("{model}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&snapshot).unwrap())
        .expect("write snapshot");
    eprintln!("\n  Baseline saved \u{2192} {}", path.display());
    eprintln!(
        "  Mean gap: {:.1}%  LSD: {:.1} dB",
        avg("mean_gap"),
        avg("spectral_gap")
    );
}

pub fn cmd_compare_models(server: &str, model: &str, ref_path: &PathBuf) {
    use foni_analyse::{analyse, compute_gap, decode_wav, spectral_timeline, TargetTensor};
    use owo_colors::OwoColorize;
    use std::io::Write;
    use tabled::{settings::Style, Table, Tabled};

    let baseline_path = data_dir().join("baselines").join(format!("{model}.json"));
    let baseline: serde_json::Value = match std::fs::read_to_string(&baseline_path) {
        Ok(s) => serde_json::from_str(&s).expect("parse baseline"),
        Err(_) => {
            eprintln!("  No baseline found at {}", baseline_path.display());
            eprintln!("  Run: fonictl snapshot {model} --vs <reference.wav>");
            return;
        }
    };

    let ref_bytes = std::fs::read(ref_path).expect("cannot read reference");
    let ref_wav = decode_wav(&ref_bytes).expect("reference WAV");
    let ref_an = analyse(&ref_wav.samples, ref_wav.sample_rate);
    let tensor = TargetTensor::from_analysis(&ref_an, "ref");

    let old_agg = &baseline["aggregate"];
    let mut new_gaps = Vec::new();
    let mut new_lsds = Vec::new();
    let mut new_presences = Vec::new();
    let mut new_brightnesses = Vec::new();
    let mut new_worst_frames = Vec::new();

    for (i, phrase) in SNAPSHOT_PHRASES.iter().enumerate() {
        eprint!("  [{}/{}] ", i + 1, SNAPSHOT_PHRASES.len());
        std::io::stderr().flush().ok();
        let wav = match synth_request(
            server,
            phrase,
            model,
            "ru",
            135,
            false,
            serde_json::json!({}),
        ) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("skip: {e}");
                continue;
            }
        };
        let decoded = decode_wav(&wav).expect("synth WAV");
        let an = analyse(&decoded.samples, decoded.sample_rate);
        let gap = compute_gap(phrase, &an, &tensor);
        let tl = spectral_timeline::compare(
            &ref_wav.samples,
            &decoded.samples,
            ref_wav.sample_rate,
            &ref_an.f0_contour,
            &an.f0_contour,
            &ref_an.energy_envelope,
            &an.energy_envelope,
        );
        eprintln!("gap {:.1}%", gap.mean_gap_pct);
        new_gaps.push(gap.mean_gap_pct);
        new_lsds.push(tl.spectral_gap);
        new_presences.push(an.pitch.voice_presence);
        new_brightnesses.push(an.spectral.brightness_hz);
        new_worst_frames.push(tl.worst_frames.first().map(|f| f.1).unwrap_or(0.0));
    }

    let n = new_gaps.len() as f32;
    let new_gap = new_gaps.iter().sum::<f32>() / n;
    let new_lsd = new_lsds.iter().sum::<f32>() / n;
    let new_pres = new_presences.iter().sum::<f32>() / n;
    let new_bright = new_brightnesses.iter().sum::<f32>() / n;
    let new_worst = new_worst_frames.iter().cloned().fold(0.0f32, f32::max);

    let old_gap = old_agg["mean_gap"].as_f64().unwrap_or(100.0) as f32;
    let old_lsd = old_agg["spectral_gap"].as_f64().unwrap_or(100.0) as f32;
    let old_pres = old_agg["voice_presence"].as_f64().unwrap_or(0.0) as f32;
    let old_bright = old_agg["brightness"].as_f64().unwrap_or(5000.0) as f32;
    let old_worst = old_agg["worst_frame"].as_f64().unwrap_or(100.0) as f32;

    #[derive(Tabled)]
    struct Row {
        #[tabled(rename = "Metric")]
        metric: &'static str,
        #[tabled(rename = "Old model")]
        old: String,
        #[tabled(rename = "New model")]
        new: String,
        #[tabled(rename = "Delta")]
        delta: String,
        #[tabled(rename = "Pass?")]
        pass: String,
    }

    let arrow = |old: f32, new: f32, lower_is_better: bool, tolerance: f32| -> (String, bool) {
        let d = new - old;
        let better = if lower_is_better { d < 0.0 } else { d > 0.0 };
        let s = if better {
            format!("{d:+.1}").green().to_string()
        } else {
            format!("{d:+.1}").red().to_string()
        };
        let ok = better || d.abs() < tolerance;
        (s, ok)
    };

    let (d1, p1) = arrow(old_gap, new_gap, true, 2.0);
    let (d2, p2) = arrow(old_lsd, new_lsd, true, 2.0);
    let (d3, p3) = arrow(old_pres, new_pres, false, 0.05);
    let (d4, p4) = arrow(old_bright, new_bright, true, 100.0);
    let (d5, p5) = arrow(old_worst, new_worst, true, 30.0);

    let rows = vec![
        Row {
            metric: "Mean gap",
            old: format!("{old_gap:.1}%"),
            new: format!("{new_gap:.1}%"),
            delta: d1,
            pass: if p1 {
                "\u{2705}".into()
            } else {
                "\u{274c}".into()
            },
        },
        Row {
            metric: "Spectral gap",
            old: format!("{old_lsd:.1} dB"),
            new: format!("{new_lsd:.1} dB"),
            delta: d2,
            pass: if p2 {
                "\u{2705}".into()
            } else {
                "\u{274c}".into()
            },
        },
        Row {
            metric: "Voice presence",
            old: format!("{old_pres:.2}"),
            new: format!("{new_pres:.2}"),
            delta: d3,
            pass: if p3 {
                "\u{2705}".into()
            } else {
                "\u{274c}".into()
            },
        },
        Row {
            metric: "Brightness",
            old: format!("{old_bright:.0} Hz"),
            new: format!("{new_bright:.0} Hz"),
            delta: d4,
            pass: if p4 {
                "\u{2705}".into()
            } else {
                "\u{274c}".into()
            },
        },
        Row {
            metric: "Worst frame",
            old: format!("{old_worst:.1} dB"),
            new: format!("{new_worst:.1} dB"),
            delta: d5,
            pass: if p5 {
                "\u{2705}".into()
            } else {
                "\u{274c}".into()
            },
        },
    ];

    let all_pass = p1 && p2 && p3 && p4 && p5;
    println!("{}", Table::new(&rows).with(Style::rounded()));
    if all_pass {
        eprintln!(
            "\n  {} New model is better. Ship it.",
            "PASS".green().bold()
        );
    } else {
        eprintln!(
            "\n  {} New model regressed on one or more metrics. Keep the old one.",
            "FAIL".red().bold()
        );
    }
}
