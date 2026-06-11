//! Real API round-trip benchmark — measures actual TTS latency and scaling.
//!
//! Hits the live foni-synth → Chatterbox pipeline.
//! Reports per-chunk RTT, sequential vs parallel throughput,
//! and whether the jitter buffer would need filler.

use std::time::{Duration, Instant};

use foni_synth::engine::jitter::{Action, JitterTracker, Trip};
use tracing::info;

const BENCH_PHRASES: &[&str] = &[
    "The Emperor protects.",
    "We shall know no fear.",
    "Victory is measured in blood.",
    "Stand firm, brothers.",
    "For the glory of the Imperium.",
    "The warp holds no terror for us.",
    "Courage and honour.",
    "Purge the unclean.",
];

#[derive(Debug)]
struct BenchResult {
    mode: String,
    chunks: usize,
    total_ms: u64,
    rtts_ms: Vec<u64>,
    filler_needed: usize,
    filler_total_ms: f64,
}

impl BenchResult {
    fn mean_rtt_ms(&self) -> f64 {
        if self.rtts_ms.is_empty() {
            return 0.0;
        }
        self.rtts_ms.iter().sum::<u64>() as f64 / self.rtts_ms.len() as f64
    }

    fn max_rtt_ms(&self) -> u64 {
        self.rtts_ms.iter().copied().max().unwrap_or(0)
    }

    fn min_rtt_ms(&self) -> u64 {
        self.rtts_ms.iter().copied().min().unwrap_or(0)
    }
}

pub fn cmd_bench_roundtrip(server: &str, chunks: usize, parallel: bool) -> Result<(), String> {
    let phrases: Vec<&str> = BENCH_PHRASES.iter().cycle().take(chunks).copied().collect();

    let client = foni_client::FoniClient::new(server);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio: {e}"))?;

    info!(chunks, parallel, server, "starting round-trip benchmark");

    // Warm up — first request wakes the container
    info!("warming up (first request may wake container)");
    let t0 = Instant::now();
    let req = foni_client::SynthRequest::new("test");
    rt.block_on(client.synthesize(&req))
        .map_err(|e| format!("warmup failed: {e}"))?;
    let warmup_ms = t0.elapsed().as_millis() as u64;
    info!(warmup_ms, "container warm");

    let result = if parallel {
        rt.block_on(bench_parallel(&client, &phrases))
    } else {
        rt.block_on(bench_sequential(&client, &phrases))
    }?;

    // Report
    println!();
    println!("  Round-Trip Benchmark");
    println!("  {}", "─".repeat(40));
    println!("  Mode:       {}", result.mode);
    println!("  Chunks:     {}", result.chunks);
    println!("  Total:      {}ms", result.total_ms);
    println!("  Mean RTT:   {:.0}ms", result.mean_rtt_ms());
    println!("  Min RTT:    {}ms", result.min_rtt_ms());
    println!("  Max RTT:    {}ms", result.max_rtt_ms());
    println!(
        "  Throughput: {:.1} chunks/min",
        result.chunks as f64 / (result.total_ms as f64 / 60_000.0)
    );
    println!();
    println!("  Jitter Analysis (0.5s playback budget)");
    println!("  {}", "─".repeat(40));
    println!(
        "  Filler needed: {} of {} chunks",
        result.filler_needed, result.chunks
    );
    println!("  Filler total:  {:.0}ms", result.filler_total_ms);
    println!();

    // Per-chunk detail
    println!("  Per-chunk RTT:");
    for (i, rtt) in result.rtts_ms.iter().enumerate() {
        let bar_len = (*rtt / 200) as usize;
        let bar = "█".repeat(bar_len.min(30));
        println!("    {i:3}  {rtt:5}ms  {bar}");
    }

    Ok(())
}

async fn bench_sequential(
    client: &foni_client::FoniClient,
    phrases: &[&str],
) -> Result<BenchResult, String> {
    let mut tracker = JitterTracker::new();
    let mut rtts = Vec::new();
    let mut filler_count = 0;
    let mut filler_total = 0.0;

    let t_total = Instant::now();

    for (i, phrase) in phrases.iter().enumerate() {
        let action = tracker.decide(0.5);
        if let Action::Filler { gap_ms } = &action {
            filler_count += 1;
            filler_total += gap_ms;
            tracker.record_filler(*gap_ms);
        }

        info!(
            chunk = i + 1,
            total = phrases.len(),
            action = ?action,
            "synth"
        );

        let t0 = Instant::now();
        let req = foni_client::SynthRequest::new(*phrase);
        let wav = client
            .synthesize(&req)
            .await
            .map_err(|e| format!("chunk {i}: {e}"))?;
        let rtt = t0.elapsed();
        let rtt_ms = rtt.as_millis() as u64;

        rtts.push(rtt_ms);
        tracker.record(Trip {
            chunk_index: i,
            rtt,
            audio_bytes: wav.0.len(),
        });
    }

    Ok(BenchResult {
        mode: "sequential".into(),
        chunks: phrases.len(),
        total_ms: t_total.elapsed().as_millis() as u64,
        rtts_ms: rtts,
        filler_needed: filler_count,
        filler_total_ms: filler_total,
    })
}

async fn bench_parallel(
    client: &foni_client::FoniClient,
    phrases: &[&str],
) -> Result<BenchResult, String> {
    let t_total = Instant::now();

    let mut handles = Vec::new();
    for (i, phrase) in phrases.iter().enumerate() {
        let client = foni_client::FoniClient::new(client.base_url());
        let phrase = phrase.to_string();
        handles.push(tokio::spawn(async move {
            let t0 = Instant::now();
            let req = foni_client::SynthRequest::new(&phrase);
            let wav = client.synthesize(&req).await;
            let rtt = t0.elapsed();
            (i, rtt, wav)
        }));
    }

    let mut results: Vec<(usize, Duration, Result<foni_client::WavData, _>)> = Vec::new();
    for handle in handles {
        results.push(handle.await.map_err(|e| format!("join: {e}"))?);
    }
    results.sort_by_key(|(i, _, _)| *i);

    let mut tracker = JitterTracker::new();
    let mut rtts = Vec::new();
    let mut filler_count = 0;
    let mut filler_total = 0.0;

    for (i, rtt, wav_result) in &results {
        let wav = wav_result.as_ref().map_err(|e| format!("chunk {i}: {e}"))?;
        let rtt_ms = rtt.as_millis() as u64;
        rtts.push(rtt_ms);

        tracker.record(Trip {
            chunk_index: *i,
            rtt: *rtt,
            audio_bytes: wav.0.len(),
        });

        let action = tracker.decide(0.5);
        if let Action::Filler { gap_ms } = action {
            filler_count += 1;
            filler_total += gap_ms;
        }
    }

    Ok(BenchResult {
        mode: "parallel".into(),
        chunks: phrases.len(),
        total_ms: t_total.elapsed().as_millis() as u64,
        rtts_ms: rtts,
        filler_needed: filler_count,
        filler_total_ms: filler_total,
    })
}

pub fn cmd_tts_stats() -> Result<(), String> {
    let output = std::process::Command::new("modal")
        .args(["container", "list", "--json"])
        .output()
        .map_err(|e| format!("modal cli: {e}"))?;

    let containers: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).unwrap_or_default();

    let tts_containers: Vec<_> = containers
        .iter()
        .filter(|c| {
            c["app_description"]
                .as_str()
                .map(|s| s.contains("foni-tts"))
                .unwrap_or(false)
        })
        .collect();

    println!("  TTS Scaling Status");
    println!("  {}", "─".repeat(30));
    println!("  Active containers: {}", tts_containers.len());
    for c in &tts_containers {
        println!(
            "    {} ({})",
            c["task_id"].as_str().unwrap_or("?"),
            c["status"].as_str().unwrap_or("?")
        );
    }
    Ok(())
}

pub fn cmd_tts_scale(max: Option<u32>, buffer: Option<u32>) -> Result<(), String> {
    info!(max = max, buffer = buffer, "scaling TTS via modal deploy");

    let status = std::process::Command::new("modal")
        .args(["deploy", "training/modal-tts-serve.py"])
        .status()
        .map_err(|e| format!("modal deploy: {e}"))?;

    if !status.success() {
        return Err("modal deploy failed".into());
    }

    println!("  Scaling updated (redeployed)");
    println!("  {}", "─".repeat(30));
    println!("  Edit training/modal-tts-serve.py to change:");
    println!("    max_containers={}", max.unwrap_or(5));
    println!("    buffer_containers={}", buffer.unwrap_or(1));
    Ok(())
}

// ── probe ─────────────────────────────────────────────────────────────────────

/// Single-shot warmness check against the Modal TTS endpoint.
pub fn cmd_probe() -> Result<(), String> {
    let url = std::env::var("FONI_TTS_URL")
        .or_else(|_| std::env::var("FISH_SPEECH_URL"))
        .unwrap_or_else(|_| "https://dpopsuev--foni-tts-serve-chatterboxtts-tts.modal.run".into());
    let token = std::env::var("FONI_TTS_TOKEN").unwrap_or_default();

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;

    let body = serde_json::json!({"text": "Да.", "language": "ru", "token": token});

    let t0 = Instant::now();
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .map_err(|e| format!("request failed: {e}"))?;
    let rtt_ms = t0.elapsed().as_millis();

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    // Heuristic: warm containers respond in < 8s; cold takes 15-30s.
    let warm = rtt_ms < 8_000;
    let dot = if warm { "●" } else { "○" };
    let state = if warm { "warm" } else { "cold" };
    println!("{dot}  {state}  {rtt_ms}ms  {url}");
    Ok(())
}

// ── dsp ──────────────────────────────────────────────────────────────────────

/// Show or reload the live DSP config from the server.
pub fn cmd_dsp(server: &str, reload: bool) -> Result<(), String> {
    if reload {
        let resp = reqwest::blocking::Client::new()
            .post(format!("{server}/controller"))
            .json(&serde_json::json!({"reload": true}))
            .send()
            .map_err(|e| format!("request: {e}"))?;
        let body: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return Ok(());
    }

    let body = super::cmd_common::get_json(server, "/controller")?;

    // Print DSP defaults and controller targets in a readable form.
    if let Some(dsp) = body.get("dsp_defaults").and_then(|v| v.as_object()) {
        println!("  DSP defaults");
        println!("  {}", "─".repeat(38));
        let mut keys: Vec<_> = dsp.keys().collect();
        keys.sort();
        for k in keys {
            let v = &dsp[k];
            if v.as_f64().is_some_and(|f| f != 0.0) || v.as_f64().is_none() {
                println!("    {k:<26} {v}");
            }
        }
    }

    if let Some(targets) = body.get("targets").and_then(|v| v.as_object()) {
        println!("\n  Controller targets");
        println!("  {}", "─".repeat(38));
        let mut keys: Vec<_> = targets.keys().collect();
        keys.sort();
        for k in keys {
            println!("    {k:<26} {}", targets[k]);
        }
    }

    let enabled = body
        .get("controller")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    println!("\n  controller: {}", if enabled { "on" } else { "off" });
    Ok(())
}

// ── cache ─────────────────────────────────────────────────────────────────────

/// Flush the server-side WAV LRU cache.
pub fn cmd_cache_clear(server: &str) -> Result<(), String> {
    let resp = reqwest::blocking::Client::new()
        .delete(format!("{server}/cache"))
        .send()
        .map_err(|e| format!("request: {e}"))?;

    let body: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
    let cleared = body.get("cleared").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("  cache cleared  ({cleared} entries removed)");
    Ok(())
}
