//! Concurrency and scheduling tests for the ONNX session pool.
//!
//! Requires foni-synth running on FONI_TEST_URL (default http://localhost:5050)
//! with the bandit model loaded.
//!
//! Run: cargo test -p foni-synth --test pool_concurrency -- --nocapture
use std::sync::Arc;
use std::time::{Duration, Instant};

const PHRASES: &[&str] = &[
    "Слушай, сталкер.",
    "Что тебе нужно?",
    "Денег нет — проваливай.",
    "Хабар принёс?",
    "Говори быстрее.",
    "Не трать моё время.",
    "Готово, забирай.",
    "Удачи, браток.",
];

fn server_url() -> String {
    std::env::var("FONI_TEST_URL").unwrap_or_else(|_| "http://localhost:5050".into())
}

async fn is_server_up(url: &str) -> bool {
    reqwest::get(format!("{url}/params"))
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn synthesize(client: &reqwest::Client, url: &str, text: &str) -> Duration {
    let t0 = Instant::now();
    client
        .post(format!("{url}/synthesize"))
        .json(&serde_json::json!({
            "text": text, "model": "bandit",
            "prosody": false, "dsp": false
        }))
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("request failed")
        .bytes()
        .await
        .expect("body failed");
    t0.elapsed()
}

async fn get_metrics(client: &reqwest::Client, url: &str) -> serde_json::Value {
    client
        .get(format!("{url}/metrics"))
        .send()
        .await
        .expect("metrics request failed")
        .json()
        .await
        .expect("metrics json failed")
}

// ── 1. Throughput scales with pool size ───────────────────────────────────────

#[tokio::test]
async fn throughput_is_faster_than_serial() {
    let url = server_url();
    let client = reqwest::Client::new();
    if !is_server_up(&url).await {
        println!("server not reachable — skipping");
        return;
    }

    // Use distinct phrases so no cache hits.
    let n = 4usize;
    let phrases: Vec<String> = (0..n)
        .map(|i| format!("Параллельный тест номер {}.", i + 1))
        .collect();

    // Warm up (load model, fill caches internally).
    let _ = synthesize(&client, &url, "прогрев").await;

    // Serial baseline.
    let t_serial = Instant::now();
    for p in &phrases {
        synthesize(&client, &url, p).await;
    }
    let serial_wall = t_serial.elapsed();

    // Parallel: fire all at once.
    let client = Arc::new(client);
    let t_parallel = Instant::now();
    let mut handles = Vec::new();
    for p in phrases {
        let c = client.clone();
        let u = url.clone();
        handles.push(tokio::spawn(async move { synthesize(&c, &u, &p).await }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let parallel_wall = t_parallel.elapsed();

    println!("serial={serial_wall:?}  parallel={parallel_wall:?}");

    // With a pool of size ≥ 4 the parallel wall time should be < serial.
    let metrics = get_metrics(&client, &url).await;
    let pool_size = metrics["pool_size"].as_u64().unwrap_or(1) as usize;

    if pool_size >= n {
        assert!(
            parallel_wall < serial_wall,
            "parallel ({parallel_wall:?}) should be faster than serial ({serial_wall:?}) \
             with pool_size={pool_size}"
        );
    } else {
        println!("pool_size={pool_size} < {n}: partial speedup only, skipping strict assertion");
    }
}

// ── 2. Pool exhaustion — N+pool_size requests all complete ────────────────────

#[tokio::test]
async fn pool_exhaustion_no_deadlock() {
    let url = server_url();
    let client = Arc::new(reqwest::Client::new());
    if !is_server_up(&url).await {
        println!("server not reachable — skipping");
        return;
    }

    let metrics = get_metrics(&client, &url).await;
    let pool_size = metrics["pool_size"].as_u64().unwrap_or(4) as usize;
    let n = pool_size + 4; // deliberately more than the pool

    let mut handles = Vec::new();
    for i in 0..n {
        let c = client.clone();
        let u = url.clone();
        handles.push(tokio::spawn(async move {
            let phrase = format!("Очередь {i}.");
            synthesize(&c, &u, &phrase).await
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let completed = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(
        completed, n,
        "all {n} requests must complete without deadlock"
    );
    println!("pool_size={pool_size} handled {n} concurrent requests — no deadlock");
}

// ── 3. Cache hits bypass the pool entirely ────────────────────────────────────

#[tokio::test]
async fn cache_hits_are_fast() {
    let url = server_url();
    let client = Arc::new(reqwest::Client::new());
    if !is_server_up(&url).await {
        println!("server not reachable — skipping");
        return;
    }

    let phrase = "Кэш-тест: быстрый путь.";

    // First call fills cache.
    synthesize(&client, &url, phrase).await;

    // Subsequent calls should hit the cache regardless of pool pressure.
    let n = 8usize;
    let t0 = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..n {
        let c = client.clone();
        let u = url.clone();
        let p = phrase.to_string();
        handles.push(tokio::spawn(async move { synthesize(&c, &u, &p).await }));
    }
    futures::future::join_all(handles).await;
    let wall = t0.elapsed();

    // 8 cache hits should complete in well under 1s total (no ONNX involved).
    println!("{n} cache hits wall time: {wall:?}");
    assert!(
        wall < Duration::from_secs(2),
        "{n} cache hits took {wall:?} — expected < 2s"
    );
}

// ── 4. Latency percentiles are reported ──────────────────────────────────────

#[tokio::test]
async fn metrics_report_latencies() {
    let url = server_url();
    let client = reqwest::Client::new();
    if !is_server_up(&url).await {
        println!("server not reachable — skipping");
        return;
    }

    synthesize(&client, &url, "Замер задержки.").await;

    let m = get_metrics(&client, &url).await;
    println!("metrics: {}", serde_json::to_string_pretty(&m).unwrap());

    assert!(
        m["p50_ms"].as_u64().unwrap_or(0) > 0,
        "p50 should be positive after synthesis"
    );
    assert!(
        m["pool_size"].as_u64().unwrap_or(0) >= 1,
        "pool_size must be ≥ 1"
    );
    assert!(
        m["requests_total"].as_u64().unwrap_or(0) >= 1,
        "requests_total must be ≥ 1"
    );
}

// ── 5. Scheduling: concurrent latencies don't diverge wildly ─────────────────

#[tokio::test]
async fn concurrent_latency_is_bounded() {
    let url = server_url();
    let client = Arc::new(reqwest::Client::new());
    if !is_server_up(&url).await {
        println!("server not reachable — skipping");
        return;
    }

    let metrics = get_metrics(&client, &url).await;
    let pool_size = metrics["pool_size"].as_u64().unwrap_or(4) as usize;
    let n = pool_size; // exactly fill the pool

    let mut handles = Vec::new();
    let t_wall = Instant::now();
    for i in 0..n {
        let c = client.clone();
        let u = url.clone();
        handles.push(tokio::spawn(async move {
            let phrase = format!("Задержка слот {}.", i);
            synthesize(&c, &u, &phrase).await
        }));
    }

    let durations: Vec<Duration> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    let wall = t_wall.elapsed();
    let max_lat = durations.iter().max().unwrap();
    let min_lat = durations.iter().min().unwrap();

    println!("pool_size={pool_size}  wall={wall:?}  min={min_lat:?}  max={max_lat:?}");

    // Max individual latency should not exceed 4× min (FIFO, no starvation).
    assert!(
        max_lat.as_millis() < min_lat.as_millis() * 4 + 500,
        "starvation detected: max={max_lat:?}  min={min_lat:?}"
    );
}
