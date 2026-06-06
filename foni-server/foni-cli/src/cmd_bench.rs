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
