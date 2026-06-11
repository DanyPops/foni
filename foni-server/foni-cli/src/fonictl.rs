mod cmd_bench;
mod cmd_commentary;
mod cmd_common;
mod cmd_data;
mod cmd_quality;
mod cmd_render;
mod cmd_sweep_shades;
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
enum CacheCmd {
    /// Flush all cached WAV entries
    Clear,
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

    /// Analyze speech emotion — print arousal/dominance/valence and mapped expression knobs
    Tone {
        /// WAV file to analyze
        file: PathBuf,
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
        /// Who you're speaking to (e.g. "Julian")
        #[arg(short, long)]
        audience: Option<String>,
    },

    /// Full voice loop: file(s) or mic → transcribe → think → speak
    Reply {
        /// Audio file(s) to respond to (skips mic recording). Multiple files concatenated.
        #[arg(short, long)]
        file: Vec<PathBuf>,
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
        /// Max recording seconds (mic mode)
        #[arg(long, default_value_t = 30)]
        max_secs: u32,
        /// Who you're speaking to
        #[arg(short, long)]
        audience: Option<String>,
    },

    /// Continuous conversation — speak naturally, pauses become chunk boundaries
    Converse {
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
        /// Who you're speaking to
        #[arg(short, long)]
        audience: Option<String>,
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
        /// Language code
        #[arg(long, default_value = "ru")]
        voice: String,
        /// Speech speed (WPM)
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
        /// Excitement: calm (0.3) → dramatic (1.5)
        #[arg(long)]
        excitement: Option<f32>,
        /// Assertiveness: tentative (0.6) → commanding (0.2)
        #[arg(long)]
        assertiveness: Option<f32>,
        /// Warmth: tense (0.4) → friendly (1.2)
        #[arg(long)]
        warmth: Option<f32>,
        /// Stress annotation backend: dict | ruaccent | none
        #[arg(long)]
        stress: Option<String>,
        /// Reference WAV for zero-shot voice cloning (base64-encoded and forwarded to Chatterbox)
        #[arg(long)]
        audio_prompt: Option<PathBuf>,
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

    /// Batch-generate comparison set
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
        /// Language / model
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

    /// Generate a contextual character commentary injection via Ollama.
    Commentary {
        /// Text to inject into (Russian sentence).
        text: String,
        /// Emotional context label passed to the LLM.
        #[arg(long, default_value = "neutral")]
        emotion: String,
        /// Ollama base URL.
        #[arg(
            long,
            env = "FONI_OLLAMA_URL",
            default_value = "http://localhost:11434"
        )]
        ollama_url: String,
        /// Ollama model name.
        #[arg(long, env = "FONI_OLLAMA_MODEL", default_value = "qwen3:1.7b")]
        model: String,
        /// Timeout for the Ollama call in milliseconds.
        #[arg(long, default_value_t = 8000)]
        timeout_ms: u64,
        /// Path to a custom lexicon.yaml with character_seed.
        #[arg(long)]
        lexicon: Option<PathBuf>,
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

    /// Sweep expression parameter space to discover perceptually distinct shades
    /// Render a manifest — synthesize each beat with its shade, concat to one file
    Render {
        /// Path to manifest JSON
        manifest: PathBuf,
        /// Output WAV file
        #[arg(short, long, default_value = "output/rendered.wav")]
        out: PathBuf,
        /// Play after rendering
        #[arg(short, long)]
        play: bool,
        /// Max concurrent synthesis requests (default matches Modal max_containers)
        #[arg(long, default_value_t = 5)]
        concurrency: usize,
    },

    /// Real API round-trip benchmark — sequential vs parallel, jitter analysis
    Bench {
        /// Number of chunks to synthesize
        #[arg(long, default_value_t = 4)]
        chunks: usize,
        /// Fire all chunks in parallel
        #[arg(long)]
        parallel: bool,
    },

    /// Query Modal TTS scaling status (backlog, runners)
    TtsStats,

    /// Show Modal inference cost ledger
    Cost,

    /// Check Modal TTS backend warmness — single ping, reports ○ cold or ● warm
    Probe,

    /// Show or reload the live DSP config
    Dsp {
        /// Reload config from dsp-defaults.json without restarting
        #[arg(long)]
        reload: bool,
    },

    /// Manage the server-side WAV cache
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
    },

    /// Update Modal TTS scaling (max containers, buffer)
    TtsScale {
        /// Max concurrent GPU containers
        #[arg(long)]
        max: Option<u32>,
        /// Warm buffer containers
        #[arg(long)]
        buffer: Option<u32>,
    },

    SweepShades {
        /// Steps per axis (3 = 27 combos, 4 = 64)
        #[arg(long, default_value_t = 3)]
        steps: usize,
        /// Output directory for WAV samples
        #[arg(short, long, default_value = "output/shade-sweep")]
        out: PathBuf,
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

// ─── Subcommand handlers ──────────────────────────────────────────────────────

// ─── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fonictl=info,foni_synth=info,foni_analyse=info".into()),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

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
                tracing::error!("{e}");
            }
        }
        Cmd::Transcribe { file, lang, model } => {
            if let Err(e) = cmd_voice::cmd_transcribe(file.as_deref(), &lang, &model) {
                tracing::error!("{e}");
            }
        }
        Cmd::Tone { file } => {
            if let Err(e) = cmd_voice::read_tone(&file) {
                tracing::error!("{e}");
            }
        }
        Cmd::Think {
            text,
            persona,
            model,
            ollama_url,

            audience,
        } => {
            let ctx = cmd_voice::VoiceContext {
                domain: None,
                audience: audience.clone(),
            };
            if let Err(e) =
                cmd_voice::cmd_think(text.as_deref(), &persona, &model, &ollama_url, &ctx)
            {
                tracing::error!("{e}");
            }
        }
        Cmd::Converse {
            persona,
            lang,
            llm,
            ollama_url,

            audience,
        } => {
            let ctx = cmd_voice::VoiceContext {
                domain: None,
                audience: audience.clone(),
            };
            if let Err(e) =
                cmd_voice::cmd_converse(server, &persona, &lang, &llm, &ollama_url, &ctx)
            {
                tracing::error!("{e}");
            }
        }
        Cmd::Reply {
            file,
            persona,
            lang,
            llm,
            ollama_url,
            max_secs,
            audience,
        } => {
            let ctx = cmd_voice::VoiceContext {
                domain: None,
                audience: audience.clone(),
            };
            if let Err(e) = cmd_voice::cmd_reply(
                server,
                &persona,
                &lang,
                &llm,
                &ollama_url,
                max_secs,
                &ctx,
                &file,
            ) {
                tracing::error!("{e}");
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
            excitement,
            assertiveness,
            warmth,
            stress,
            audio_prompt,
        } => {
            let text = cmd_voice::resolve_text(text);
            let text = match &text {
                Some(t) => t,
                None => {
                    tracing::warn!("✗ no text provided (pass argument or pipe via stdin)");
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
                excitement,
                assertiveness,
                warmth,
                stress.as_deref(),
                audio_prompt.as_ref(),
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
                tracing::error!("{e}");
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
                tracing::error!("{e}");
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
                tracing::error!("{e}");
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
                tracing::error!("{e}");
            }
        }
        Cmd::Commentary {
            text,
            emotion,
            ollama_url,
            model,
            timeout_ms,
            lexicon,
        } => {
            tokio::runtime::Runtime::new()
                .expect("tokio runtime")
                .block_on(cmd_commentary::cmd_commentary(
                    &text,
                    &emotion,
                    &ollama_url,
                    &model,
                    timeout_ms,
                    lexicon.as_ref(),
                ));
        }
        Cmd::TtsBench { url, phrase } => {
            cmd_train::cmd_tts_bench(&url, &phrase);
        }
        Cmd::TtsCompare { phrase } => {
            cmd_train::cmd_tts_compare(&phrase);
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
                tracing::error!("{e}");
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
                tracing::error!("{e}");
            }
        }
        Cmd::CompareModels { model, vs } => {
            if let Err(e) = cmd_train::cmd_compare_models(server, &model, &vs) {
                tracing::error!("{e}");
            }
        }
        Cmd::Calibrate { text, vs, model } => {
            cmd_quality::cmd_calibrate(server, &text, &vs, &model);
        }
        Cmd::Bench { chunks, parallel } => {
            if let Err(e) = cmd_bench::cmd_bench_roundtrip(server, chunks, parallel) {
                tracing::error!("{e}");
            }
        }
        Cmd::TtsStats => {
            if let Err(e) = cmd_bench::cmd_tts_stats() {
                tracing::error!("{e}");
            }
        }
        Cmd::Cost => {
            cost::print_summary();
        }
        Cmd::Probe => {
            if let Err(e) = cmd_bench::cmd_probe() {
                tracing::error!("{e}");
            }
        }
        Cmd::Dsp { reload } => {
            if let Err(e) = cmd_bench::cmd_dsp(server, reload) {
                tracing::error!("{e}");
            }
        }
        Cmd::Cache { cmd } => match cmd {
            CacheCmd::Clear => {
                if let Err(e) = cmd_bench::cmd_cache_clear(server) {
                    tracing::error!("{e}");
                }
            }
        },
        Cmd::TtsScale { max, buffer } => {
            if let Err(e) = cmd_bench::cmd_tts_scale(max, buffer) {
                tracing::error!("{e}");
            }
        }
        Cmd::Render {
            manifest,
            out,
            play,
            concurrency,
        } => {
            if let Err(e) = cmd_render::cmd_render(server, &manifest, &out, play, concurrency) {
                tracing::error!("{e}");
            }
        }
        Cmd::SweepShades { steps, out } => {
            if let Err(e) = cmd_sweep_shades::cmd_sweep_shades(server, steps, &out) {
                tracing::error!("{e}");
            }
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
