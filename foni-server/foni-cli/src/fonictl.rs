pub mod cloud;
mod cmd_common;
mod cmd_data;
mod cmd_quality;
mod cmd_synth;
mod cmd_train;
mod cmd_tune;
mod cmd_voice;
pub mod cost;
pub mod modal_cloud;
mod tui;

use cmd_common::{cache_dir, data_dir, play_wav};
use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    /// Record from microphone until silence, print WAV path to stdout
    Rec {
        /// Output WAV file
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Silence threshold in dB to stop recording (default: -30)
        #[arg(long, default_value_t = -30, allow_negative_numbers = true)]
        silence_db: i32,
        /// Seconds of silence before stopping (default: 1.5)
        #[arg(long, default_value_t = 1.5)]
        silence_secs: f64,
        /// Maximum recording duration in seconds (default: 30)
        #[arg(long, default_value_t = 30)]
        max_secs: u32,
    },

    /// Transcribe audio to text via Whisper, print to stdout
    Transcribe {
        /// WAV file (or reads path from stdin)
        file: Option<PathBuf>,
        /// Language code
        #[arg(short, long, default_value = "en")]
        lang: String,
        /// Whisper model size
        #[arg(long, default_value = "base")]
        model: String,
    },

    /// Send text to LLM, print reply to stdout
    Think {
        /// Text to send (or reads from stdin)
        text: Option<String>,
        /// Persona system prompt
        #[arg(short, long, default_value = "diomedes")]
        persona: String,
        /// Ollama model
        #[arg(short, long, default_value = "llama3.2")]
        model: String,
        /// Ollama URL
        #[arg(long, env = "OLLAMA_URL", default_value = "http://localhost:11434")]
        ollama_url: String,
    },

    /// Full voice loop: record → transcribe → think → speak
    Reply {
        /// Persona
        #[arg(short, long, default_value = "diomedes")]
        persona: String,
        /// Whisper language
        #[arg(short, long, default_value = "en")]
        lang: String,
        /// Ollama model
        #[arg(long, default_value = "llama3.2")]
        llm: String,
        /// Ollama URL
        #[arg(long, env = "OLLAMA_URL", default_value = "http://localhost:11434")]
        ollama_url: String,
        /// Max recording seconds
        #[arg(long, default_value_t = 30)]
        max_secs: u32,
    },

    /// Synthesize text → WAV
    Synth {
        /// Text to speak (or reads from stdin)
        text: Option<String>,
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
        /// Emotion intensity (0.25–2.0, default 0.5 = neutral, 1.0+ = dramatic)
        #[arg(long)]
        exaggeration: Option<f32>,
        /// Pace weight (0.0–1.0, default 0.5, lower = slower/looser)
        #[arg(long)]
        cfg_weight: Option<f32>,
        /// Prosody randomness (0.05–5.0, default 0.8)
        #[arg(long)]
        temperature: Option<f32>,
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
        /// Where to write synthetic WAVs (default: ~/.cache/foni/compare/)
        #[arg(long)]
        out_dir: Option<PathBuf>,
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

    /// RunPod cloud GPU management — balance, GPUs, spend history
    Cloud {
        #[command(subcommand)]
        action: CloudAction,
    },

    /// Test a Rhai policy script against canned analysis data (no server needed)
    TestPolicy {
        /// Path to the .rhai script
        script: PathBuf,
        /// Simulated brightness (Hz)
        #[arg(long, default_value_t = 3400.0)]
        brightness: f32,
        /// Simulated loudness (dBFS)
        #[arg(long, default_value_t = -19.0, allow_hyphen_values = true)]
        loudness: f32,
        /// Simulated bass balance (dB)
        #[arg(long, default_value_t = 14.0)]
        bass: f32,
        /// Simulated vocal darkness (dB/oct)
        #[arg(long, default_value_t = -5.0, allow_hyphen_values = true)]
        darkness: f32,
    },

    /// Download audio from URL, convert to mono 24kHz WAV, split into clips by silence
    Fetch {
        /// YouTube or direct audio URL
        url: String,
        /// Output directory for WAV clips
        #[arg(short, long, default_value = "dataset")]
        out: PathBuf,
        /// Keep as single file instead of splitting
        #[arg(long)]
        no_split: bool,
        /// Silence threshold in dB (default: -30)
        #[arg(long, default_value_t = -30, allow_negative_numbers = true)]
        silence_db: i32,
        /// Minimum silence gap to split on, in seconds (default: 0.4)
        #[arg(long, default_value_t = 0.4)]
        min_gap: f64,
        /// Minimum clip duration in seconds (default: 0.5)
        #[arg(long, default_value_t = 0.5)]
        min_clip: f64,
        /// Maximum clip duration in seconds (default: 15)
        #[arg(long, default_value_t = 15.0)]
        max_clip: f64,
    },

    /// Clean a dataset directory — trim silence, normalize volume, report clipping
    Clean {
        /// Input directory of WAV files
        dir: PathBuf,
        /// Output directory for cleaned WAVs
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Augment a dataset — speed perturbation to expand training data
    Augment {
        /// Input directory of WAV files
        dir: PathBuf,
        /// Output directory for augmented WAVs
        #[arg(long)]
        out: Option<PathBuf>,
        /// Speed factors (comma-separated, e.g. "0.95,1.0,1.05")
        #[arg(long, default_value = "0.95,1.0,1.05")]
        speeds: String,
    },

    /// Full training pipeline — clean, augment, train on cloud GPU, compare, deploy
    Train {
        /// Model name
        #[arg(default_value = "sidorovich")]
        model: String,
        /// Dataset directory of studio WAV files
        #[arg(long, default_value = "baseline/stalker/wav/sidorovich")]
        dataset: PathBuf,
        /// Reference WAV for quality comparison
        #[arg(long, default_value = "baseline/stalker/wav/sidorovich/trader1a.wav")]
        vs: PathBuf,
        /// Training steps
        #[arg(long, default_value_t = 500)]
        steps: u32,
        /// Simulate the full pipeline without touching Modal (no cost)
        #[arg(long)]
        dry_run: bool,
        /// Unused (kept for compat)
        #[arg(long, default_value = "foni-train")]
        ntfy: String,
        /// Stream logs inline instead of fire-and-forget
        #[arg(long)]
        follow: bool,
    },

    /// Check status of a training job
    TrainStatus {
        /// Job ID from fonictl train
        call_id: String,
    },

    /// Stream logs from a training job
    TrainLogs {
        /// Job ID from fonictl train
        call_id: String,
    },

    /// Cancel a running training job
    TrainCancel {
        /// Job ID from fonictl train
        call_id: String,
    },

    /// Benchmark TTS endpoint latency (cold + warm)
    TtsBench {
        /// Endpoint URL
        #[arg(default_value = "https://dpopsuev--chatterbox.modal.run")]
        url: String,
        /// Phrase to synthesize
        #[arg(long, default_value = "Привет, сталкер. Как дела на болотах?")]
        phrase: String,
    },

    /// Per-frame RMS energy profile for a WAV file
    Energy {
        /// WAV file to analyse
        file: PathBuf,
        /// Frame size in milliseconds
        #[arg(long, default_value_t = 100)]
        frame_ms: usize,
    },

    /// Compare Chatterbox vs Fish S2-Pro on the same phrase (parallel)
    TtsCompare {
        /// Phrase to synthesize
        #[arg(
            default_value = "Слушай, сталкер. Я тут тебе ситуацию объясню. На Зоне сейчас неспокойно, аномалии активизировались. Деплой завалился, пайплайн сломался. Короче, полный пиздец. Но ты не переживай, мы всё починим. Удачи, браток."
        )]
        phrase: String,
    },

    /// Save current model's scores as the baseline to beat before retraining
    Snapshot {
        /// Model name
        #[arg(default_value = "sidorovich")]
        model: String,
        /// Reference WAV
        #[arg(long)]
        vs: PathBuf,
    },

    /// Compare new model against saved baseline — auto pass/fail
    CompareModels {
        /// Model name (loads baseline from ~/.local/share/foni/baselines/<name>.json)
        #[arg(default_value = "sidorovich")]
        model: String,
        /// Reference WAV
        #[arg(long)]
        vs: PathBuf,
    },

    /// Sweep a knob through multiple values, show comparison table
    Sweep {
        /// Knob name
        knob: String,
        /// Values to try (comma-separated, e.g. "-6,-10,-14")
        #[arg(allow_hyphen_values = true)]
        values: String,
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

#[derive(Subcommand)]
enum CloudAction {
    /// Show account balance, endpoints, templates, lifetime spend
    Status,
    /// List available GPUs ranked by price
    Gpus,
    /// Show cost history from the local ledger
    History,
    /// Check serverless endpoint worker health
    Health,
    /// One-time setup: create template + endpoint + registry auth
    Setup {
        /// Container image to use
        #[arg(long, default_value = "ghcr.io/danypops/foni-rvc-train:latest")]
        image: String,
    },
    /// Cancel a running or queued job
    Cancel {
        /// Job ID to cancel
        job_id: String,
    },
    /// Wait for a worker to become ready, optionally notify via ntfy
    Wait {
        /// ntfy topic to notify when ready (e.g. "foni-train")
        #[arg(long)]
        ntfy: Option<String>,
        /// Max wait time in seconds
        #[arg(long, default_value = "600")]
        timeout: u64,
    },
    /// Show endpoint details (GPUs, workers, template)
    Endpoint,
    /// Update endpoint GPU types
    UpdateGpus {
        /// Comma-separated GPU type IDs
        gpus: String,
    },
    /// Delete endpoint and create a fresh one
    ResetEndpoint,
    /// Purge all queued jobs
    Purge,
    /// Submit a raw job to the endpoint
    Submit {
        /// JSON input payload
        input: String,
    },
    /// Create an on-demand pod (persistent disk, no cold start)
    CreatePod {
        /// GPU type ID (e.g. "NVIDIA RTX A5000")
        #[arg(long, default_value = "NVIDIA RTX A5000")]
        gpu: String,
        /// Container disk size in GB
        #[arg(long, default_value_t = 20)]
        disk: u32,
        /// Use RunPod's pre-cached PyTorch image (fast boot, pip install at startup)
        #[arg(long)]
        cached: bool,
    },
    /// Stop and delete a pod
    DeletePod {
        /// Pod ID
        pod_id: String,
    },
    /// List running pods
    Pods,
    /// Terminate all running pods
    KillAll,
    /// Create a training template
    CreateTemplate {
        /// Template name
        #[arg(long, default_value = "foni-train")]
        name: String,
        /// Container image
        #[arg(
            long,
            default_value = "runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04"
        )]
        image: String,
        /// Start command (run inside bash -c)
        #[arg(
            long,
            default_value = "wget -qO /finetune.py https://raw.githubusercontent.com/DanyPops/foni/master/rvc/fish-finetune.py && python3 /finetune.py; sleep 300"
        )]
        cmd: String,
    },
}

// ─── Subcommand handlers ──────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
// ─── Entry point ──────────────────────────────────────────────────────────────

/// Aggregate acoustic fingerprint across every WAV in a directory.
/// Runs in a single process — no subprocess overhead per file.

fn cmd_cloud(action: CloudAction) {
    use cloud::{CloudProvider, RunPodProvider};

    use tabled::{settings::Style, Table, Tabled};

    let api_key = match std::env::var("RUNPOD_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("  RUNPOD_API_KEY not set. Export it in your shell.");
            return;
        }
    };
    let _provider = RunPodProvider::new(&api_key);

    match action {
        CloudAction::History => {
            let ledger = cost::load();
            if ledger.receipts.is_empty() {
                eprintln!("  No training runs yet.");
                return;
            }
            #[derive(Tabled)]
            struct HistRow {
                #[tabled(rename = "Date")]
                date: String,
                #[tabled(rename = "GPU")]
                gpu: String,
                #[tabled(rename = "Duration")]
                duration: String,
                #[tabled(rename = "Cost")]
                cost: String,
                #[tabled(rename = "Model")]
                model: String,
            }
            let rows: Vec<HistRow> = ledger
                .receipts
                .iter()
                .map(|e| HistRow {
                    date: e.timestamp[..10].to_string(),
                    gpu: e.gpu.clone(),
                    duration: format!("{:.0} min", e.duration_min),
                    cost: format!("${:.2}", e.cost_usd),
                    model: e.model_name.clone(),
                })
                .collect();
            println!("{}", Table::new(&rows).with(Style::rounded()));
            println!("  Total: ${:.2}", ledger.total_cost());
            return;
        }
        _ => {}
    }

    let api_key = match std::env::var("RUNPOD_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("  RUNPOD_API_KEY not set. Export it in your shell.");
            return;
        }
    };
    let provider = RunPodProvider::new(&api_key);

    match action {
        CloudAction::Status => {
            let ledger = cost::load();
            let status = provider.balance().expect("RunPod API");
            println!("  Balance:        ${:.2}", status.balance);
            println!("  Spend/hr:       ${:.4}", status.spend_per_hr);
            println!("  Active pods:    {}", status.active_pods);

            let endpoint_id = std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_default();
            if !endpoint_id.is_empty() {
                if let Ok(ep) = provider.get_endpoint(&endpoint_id) {
                    println!(
                        "  Endpoint:       {} ({})",
                        ep["name"].as_str().unwrap_or("?"),
                        endpoint_id
                    );
                    let gpus = ep["gpuTypeIds"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|g| g.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    println!("  GPUs:           {gpus}");
                }
                if let Ok(billing) = provider.billing_endpoints() {
                    if let Some(entries) = billing.as_array() {
                        let total: f64 = entries
                            .iter()
                            .filter(|e| e["endpointId"].as_str() == Some(&endpoint_id))
                            .filter_map(|e| e["amount"].as_f64())
                            .sum();
                        if total > 0.0 {
                            println!("  RunPod billing: ${total:.4}");
                        }
                    }
                }
            }
            println!(
                "  Lifetime spend: ${:.2} ({} runs, {:.1}h GPU)",
                ledger.total_cost(),
                ledger.run_count(),
                ledger.total_gpu_hours()
            );
        }
        CloudAction::Gpus => {
            #[derive(Tabled)]
            struct GpuRow {
                #[tabled(rename = "GPU")]
                name: String,
                #[tabled(rename = "VRAM")]
                vram: String,
                #[tabled(rename = "Price/hr")]
                price: String,
            }
            let mut gpus = provider.gpu_types().expect("RunPod API");
            gpus.retain(|g| g.memory_gb >= 12 && g.community_price.is_some());
            gpus.sort_by(|a, b| {
                a.community_price
                    .partial_cmp(&b.community_price)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let rows: Vec<GpuRow> = gpus
                .iter()
                .take(15)
                .map(|g| GpuRow {
                    name: g.display_name.clone(),
                    vram: format!("{}GB", g.memory_gb),
                    price: format!("${:.2}", g.community_price.unwrap_or(0.0)),
                })
                .collect();
            println!("{}", Table::new(&rows).with(Style::rounded()));
        }
        CloudAction::Health => {
            let endpoint_id =
                std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_else(|_| "none".into());
            if endpoint_id == "none" {
                eprintln!("  FONI_RUNPOD_ENDPOINT not set.");
                return;
            }
            match provider.endpoint_health(&endpoint_id) {
                Ok(h) => println!("{}", serde_json::to_string_pretty(&h).unwrap_or_default()),
                Err(e) => eprintln!("  Health check failed: {e}"),
            }
        }
        CloudAction::Setup { image } => {
            eprintln!("  Setting up RunPod Serverless infrastructure...");

            // 1. Register ghcr.io credentials
            let gh_token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
            if !gh_token.is_empty() {
                match provider.register_registry("ghcr-foni", "DanyPops", &gh_token) {
                    Ok(id) => eprintln!("  Registry auth: {id}"),
                    Err(e) => eprintln!("  Registry auth failed: {e}"),
                }
            }

            // 2. Create template
            match provider.create_template("foni-rvc-train", &image, None) {
                Ok(id) => {
                    eprintln!("  Template: {id}");

                    // 3. Create endpoint
                    match provider.create_endpoint("foni-train", &id, "AMPERE_24", 14_400_000) {
                        Ok(eid) => {
                            eprintln!("  Endpoint: {eid}");
                            eprintln!("\n  Add to your shell:");
                            eprintln!("    export FONI_RUNPOD_ENDPOINT={eid}");
                        }
                        Err(e) => eprintln!("  Endpoint creation failed: {e}"),
                    }
                }
                Err(e) => eprintln!("  Template creation failed: {e}"),
            }
        }
        CloudAction::Cancel { job_id } => {
            let endpoint_id =
                std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_else(|_| "none".into());
            match provider.cancel_job(&endpoint_id, &job_id) {
                Ok(()) => eprintln!("  Cancelled: {job_id}"),
                Err(e) => eprintln!("  Cancel failed: {e}"),
            }
        }
        CloudAction::Wait { ntfy, timeout } => {
            let endpoint_id =
                std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_else(|_| "none".into());
            if endpoint_id == "none" {
                eprintln!("  FONI_RUNPOD_ENDPOINT not set.");
                return;
            }
            let start = std::time::Instant::now();
            let deadline = std::time::Duration::from_secs(timeout);
            let poll_interval = std::time::Duration::from_secs(10);

            loop {
                match provider.endpoint_health(&endpoint_id) {
                    Ok(h) => {
                        let workers = &h["workers"];
                        let ready = workers["ready"].as_u64().unwrap_or(0);
                        let idle = workers["idle"].as_u64().unwrap_or(0);
                        let unhealthy = workers["unhealthy"].as_u64().unwrap_or(0);
                        let init = workers["initializing"].as_u64().unwrap_or(0);

                        if ready > 0 || idle > 0 {
                            let elapsed = start.elapsed().as_secs();
                            eprintln!("  Worker ready after {elapsed}s");
                            if let Some(topic) = &ntfy {
                                let _ = reqwest::blocking::Client::new()
                                    .post(format!("https://ntfy.sh/{topic}"))
                                    .header("Title", "foni-train: worker ready")
                                    .body(format!("Worker ready after {elapsed}s. Run: fonictl train sidorovich"))
                                    .send();
                            }
                            println!("ready");
                            return;
                        }
                        if unhealthy > 0 {
                            eprintln!("  Worker unhealthy — image pull or handler failed");
                            if let Some(topic) = &ntfy {
                                let _ = reqwest::blocking::Client::new()
                                    .post(format!("https://ntfy.sh/{topic}"))
                                    .header("Title", "foni-train: worker UNHEALTHY")
                                    .header("Priority", "high")
                                    .body("Worker failed to initialize")
                                    .send();
                            }
                            println!("unhealthy");
                            return;
                        }
                        eprint!(
                            "\r  Waiting... init={init} ready={ready} [{:.0}s]",
                            start.elapsed().as_secs_f64()
                        );
                    }
                    Err(e) => eprint!("\r  Poll error: {e}"),
                }

                if start.elapsed() > deadline {
                    eprintln!("\n  Timed out after {timeout}s");
                    println!("timeout");
                    return;
                }
                std::thread::sleep(poll_interval);
            }
        }
        CloudAction::Endpoint => {
            let endpoint_id =
                std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_else(|_| "none".into());
            if endpoint_id == "none" {
                eprintln!("  FONI_RUNPOD_ENDPOINT not set.");
                return;
            }
            match provider.get_endpoint(&endpoint_id) {
                Ok(ep) => println!("{}", serde_json::to_string_pretty(&ep).unwrap_or_default()),
                Err(e) => eprintln!("  {e}"),
            }
        }
        CloudAction::UpdateGpus { gpus } => {
            let endpoint_id =
                std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_else(|_| "none".into());
            if endpoint_id == "none" {
                eprintln!("  FONI_RUNPOD_ENDPOINT not set.");
                return;
            }
            let gpu_list: Vec<&str> = gpus.split(',').map(|s| s.trim()).collect();
            match provider
                .update_endpoint(&endpoint_id, serde_json::json!({ "gpuTypeIds": gpu_list }))
            {
                Ok(ep) => {
                    let updated = ep["gpuTypeIds"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|g| g.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    eprintln!("  GPUs updated: {updated}");
                }
                Err(e) => eprintln!("  {e}"),
            }
        }
        CloudAction::ResetEndpoint => {
            let endpoint_id =
                std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_else(|_| "none".into());
            if endpoint_id == "none" {
                eprintln!("  FONI_RUNPOD_ENDPOINT not set.");
                return;
            }
            // Get current config before delete
            let ep = match provider.get_endpoint(&endpoint_id) {
                Ok(ep) => ep,
                Err(e) => {
                    eprintln!("  Cannot read endpoint: {e}");
                    return;
                }
            };
            // Purge + delete
            provider.cancel_job(&endpoint_id, "purge-queue").ok();
            let template_id = ep["templateId"].as_str().unwrap_or("hu8c3blznq");
            let gpus = ep["gpuTypeIds"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|g| g.as_str())
                        .collect::<Vec<_>>()
                        .join("\",\"")
                })
                .unwrap_or_default();
            // Delete via REST
            let _ = provider.rest_delete(&format!("/endpoints/{endpoint_id}"));
            eprintln!("  Deleted {endpoint_id}");
            // Recreate
            match provider.create_endpoint(
                ep["name"].as_str().unwrap_or("foni-train"),
                template_id,
                &gpus,
                ep["executionTimeoutMs"].as_u64().unwrap_or(14_400_000),
            ) {
                Ok(new_id) => {
                    eprintln!("  Created {new_id}");
                    eprintln!("  Update your shell: export FONI_RUNPOD_ENDPOINT={new_id}");
                }
                Err(e) => eprintln!("  Recreate failed: {e}"),
            }
        }
        CloudAction::Purge => {
            let endpoint_id =
                std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_else(|_| "none".into());
            if endpoint_id == "none" {
                eprintln!("  FONI_RUNPOD_ENDPOINT not set.");
                return;
            }
            match provider.purge_queue(&endpoint_id) {
                Ok(removed) => eprintln!("  Purged {removed} job(s)"),
                Err(e) => eprintln!("  {e}"),
            }
        }
        CloudAction::Submit { input } => {
            let endpoint_id =
                std::env::var("FONI_RUNPOD_ENDPOINT").unwrap_or_else(|_| "none".into());
            if endpoint_id == "none" {
                eprintln!("  FONI_RUNPOD_ENDPOINT not set.");
                return;
            }
            let payload: serde_json::Value = match serde_json::from_str(&input) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("  Invalid JSON: {e}");
                    return;
                }
            };
            let ntfy_topic =
                std::env::var("FONI_NTFY_TOPIC").unwrap_or_else(|_| "foni-train".into());
            match provider.submit_job(
                &endpoint_id,
                payload,
                Some(&format!("https://ntfy.sh/{ntfy_topic}")),
            ) {
                Ok(job) => {
                    println!("{}", job.id);
                    eprintln!("  Status: {}", job.status);
                }
                Err(e) => eprintln!("  {e}"),
            }
        }
        CloudAction::CreatePod {
            gpu,
            disk,
            cached: _,
        } => {
            let image = std::env::var("FONI_TRAIN_IMAGE")
                .unwrap_or_else(|_| "runpod/pytorch:1.0.2-cu1281-torch280-ubuntu2404".into());
            match provider.create_pod(cloud::CreatePodOpts {
                gpu_type_id: gpu,
                image,
                volume_gb: 0,
                container_disk_gb: disk,
                name: "foni-train".into(),
                ports: "8888/http".into(),
                docker_args: String::new(),
                template_id: None,
                env: vec![],
            }) {
                Ok(pod) => {
                    println!("{}", pod.id);
                    eprintln!("  GPU:    {}", pod.gpu_name);
                    eprintln!("  Cost:   ${:.2}/hr", pod.cost_per_hr);
                    eprintln!("  Status: {}", pod.status);
                }
                Err(e) => eprintln!("  {e}"),
            }
        }
        CloudAction::DeletePod { pod_id } => match provider.terminate_pod(&pod_id) {
            Ok(()) => eprintln!("  Deleted {pod_id}"),
            Err(e) => eprintln!("  {e}"),
        },
        CloudAction::Pods => match provider.list_pods() {
            Ok(pods) => println!(
                "{}",
                serde_json::to_string_pretty(&pods).unwrap_or_default()
            ),
            Err(e) => eprintln!("  {e}"),
        },
        CloudAction::KillAll => match provider.list_pods() {
            Ok(pods) => {
                let empty = vec![];
                let arr = pods.as_array().unwrap_or(&empty);
                if arr.is_empty() {
                    eprintln!("  No pods running");
                } else {
                    for p in arr {
                        if let Some(id) = p["id"].as_str() {
                            match provider.terminate_pod(id) {
                                Ok(()) => eprintln!("  Killed {id}"),
                                Err(e) => eprintln!("  Failed {id}: {e}"),
                            }
                        }
                    }
                }
            }
            Err(e) => eprintln!("  {e}"),
        },
        CloudAction::CreateTemplate { name, image, cmd } => {
            match provider.create_template_graphql(&name, &image, &cmd, None) {
                Ok(id) => {
                    println!("{id}");
                    eprintln!("  Template created. Set: export FONI_TEMPLATE_ID={id}");
                }
                Err(e) => eprintln!("  {e}"),
            }
        }
        CloudAction::History => unreachable!(),
    }
}

fn main() {
    let cli = Cli::parse();
    let server = cli.server.trim_end_matches('/');

    match cli.cmd {
        Cmd::Rec {
            out,
            silence_db,
            silence_secs,
            max_secs,
        } => {
            if let Err(e) = cmd_voice::cmd_rec(out.as_deref(), silence_db, silence_secs, max_secs) {
                eprintln!("✗ {e}");
            }
        }
        Cmd::Transcribe { file, lang, model } => {
            if let Err(e) = cmd_voice::cmd_transcribe(file.as_deref(), &lang, &model) {
                eprintln!("✗ {e}");
            }
        }
        Cmd::Think {
            text,
            persona,
            model,
            ollama_url,
        } => {
            if let Err(e) = cmd_voice::cmd_think(text.as_deref(), &persona, &model, &ollama_url) {
                eprintln!("✗ {e}");
            }
        }
        Cmd::Reply {
            persona,
            lang,
            llm,
            ollama_url,
            max_secs,
        } => {
            if let Err(e) =
                cmd_voice::cmd_reply(server, &persona, &lang, &llm, &ollama_url, max_secs)
            {
                eprintln!("✗ {e}");
            }
        }
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
            exaggeration,
            cfg_weight,
            temperature,
        } => {
            let text = cmd_voice::resolve_text(text);
            let text = match &text {
                Some(t) => t,
                None => {
                    eprintln!("✗ no text provided (pass argument or pipe via stdin)");
                    return;
                }
            };
            cmd_synth::cmd_synth(
                server,
                text,
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
                exaggeration,
                cfg_weight,
                temperature,
            );
        }
        Cmd::Studio { text, model, from } => {
            cmd_synth::cmd_studio(server, &text, &model, from.as_deref());
        }
        Cmd::Samples { out_dir, model } => {
            cmd_synth::cmd_samples(server, &out_dir, &model);
        }
        Cmd::Status => {
            cmd_synth::cmd_status(server);
        }
        Cmd::Process {
            file,
            out,
            opts,
            vs,
        } => {
            cmd_synth::cmd_process(server, &file, out.as_ref(), &opts, vs.as_ref());
        }
        Cmd::Play { file } => {
            play_wav(&file);
        }
        Cmd::Analyse { file, vs, timeline } => {
            if let Err(e) = cmd_quality::cmd_analyse(&file, vs.as_ref(), timeline) {
                eprintln!("✗ {e}");
            }
        }
        Cmd::Compare {
            studio,
            out_dir,
            max_dur,
            model,
            skip_transcribe,
        } => {
            let out = out_dir.unwrap_or_else(|| cache_dir().join("compare"));
            if let Err(e) =
                cmd_quality::cmd_compare(server, &studio, &out, max_dur, &model, skip_transcribe)
            {
                eprintln!("✗ {e}");
            }
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
                cmd_tune::cmd_tune_auto(
                    server,
                    &text,
                    &presets,
                    &model,
                    n_iter,
                    vs.as_deref(),
                    reference.as_deref(),
                );
            } else {
                cmd_tune::cmd_tune(server, &text, &presets, reference.as_deref(), &model);
            }
        }
        Cmd::Corpus { dir, vs } => {
            if let Err(e) = cmd_data::cmd_corpus(&dir, vs.as_ref()) {
                eprintln!("✗ {e}");
            }
        }
        Cmd::Train {
            model,
            dataset,
            vs,
            steps,
            dry_run,
            ntfy,
            follow,
        } => {
            cmd_train::cmd_train(server, &model, &dataset, &vs, steps, dry_run, &ntfy, follow);
        }
        Cmd::TrainStatus { call_id } => {
            cmd_train::cmd_train_status(&call_id);
        }
        Cmd::TrainLogs { call_id } => {
            cmd_train::cmd_train_logs(&call_id);
        }
        Cmd::TrainCancel { call_id } => {
            cmd_train::cmd_train_cancel(&call_id);
        }
        Cmd::Energy { file, frame_ms } => {
            if let Err(e) = cmd_quality::cmd_energy(&file, frame_ms) {
                eprintln!("✗ {e}");
            }
        }
        Cmd::TtsBench { url, phrase } => {
            cmd_train::cmd_tts_bench(&url, &phrase);
        }
        Cmd::TtsCompare { phrase } => {
            cmd_train::cmd_tts_compare(&phrase);
        }
        Cmd::Cloud { action } => {
            cmd_cloud(action);
        }
        Cmd::TestPolicy {
            script,
            brightness,
            loudness,
            bass,
            darkness,
        } => {
            cmd_tune::cmd_test_policy(&script, brightness, loudness, bass, darkness);
        }
        Cmd::Fetch {
            url,
            out,
            no_split,
            silence_db,
            min_gap,
            min_clip,
            max_clip,
        } => {
            let opts = cmd_data::FetchOpts {
                silence_db,
                min_gap,
                min_clip,
                max_clip,
            };
            if let Err(e) = cmd_data::cmd_fetch(&url, &out, !no_split, &opts) {
                eprintln!("✗ {e}");
            }
        }
        Cmd::Clean { dir, out } => {
            let out = out.unwrap_or_else(|| data_dir().join("training/clean"));
            cmd_data::cmd_clean(&dir, &out);
        }
        Cmd::Augment { dir, out, speeds } => {
            let out = out.unwrap_or_else(|| data_dir().join("training/augmented"));
            cmd_data::cmd_augment(&dir, &out, &speeds);
        }
        Cmd::Snapshot { model, vs } => {
            if let Err(e) = cmd_train::cmd_snapshot(server, &model, &vs) {
                eprintln!("✗ {e}");
            }
        }
        Cmd::CompareModels { model, vs } => {
            if let Err(e) = cmd_train::cmd_compare_models(server, &model, &vs) {
                eprintln!("✗ {e}");
            }
        }
        Cmd::Calibrate { text, vs, model } => {
            cmd_quality::cmd_calibrate(server, &text, &vs, &model);
        }
        Cmd::Sweep {
            knob,
            values,
            text,
            vs,
            model,
        } => {
            cmd_quality::cmd_sweep(server, &knob, &values, &text, &vs, &model);
        }
        Cmd::Diff {
            knob,
            value,
            text,
            vs,
            model,
        } => {
            cmd_quality::cmd_diff(server, &knob, value, &text, &vs, &model);
        }
        Cmd::Mix {
            text,
            model,
            from,
            reference,
        } => {
            cmd_synth::cmd_mix(server, &text, &model, from.as_deref(), reference.as_deref());
        }
        Cmd::Listen {
            text,
            model,
            dsp,
            diagnose,
            vs,
        } => {
            if diagnose {
                cmd_synth::cmd_diagnose(server, &text, &model);
            } else {
                cmd_synth::cmd_listen(server, &text, &model, dsp, vs);
            }
        }
    }
}
