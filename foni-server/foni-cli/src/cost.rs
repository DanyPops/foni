//! Persistent cost ledger — tracks Modal inference and training spend.
//!
//! Stored at $XDG_DATA_HOME/foni/cost-ledger.json. Append-only.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Modal T4 GPU price per second (on-demand, as of 2026-06).
pub const MODAL_T4_COST_PER_SEC: f64 = 0.000306;

/// A receipt for one TTS synthesis call via Modal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceReceipt {
    pub timestamp: String,
    /// Text preview (first 60 chars).
    pub text_preview: String,
    /// Number of characters synthesized.
    pub text_chars: usize,
    /// Output audio duration in seconds.
    pub audio_duration_sec: f64,
    /// Wall-clock RTT in milliseconds.
    pub rtt_ms: u64,
    /// Whether this was a cache hit (no GPU cost).
    pub cache_hit: bool,
    /// Estimated GPU seconds consumed.
    pub gpu_sec: f64,
    /// Estimated cost in USD.
    pub cost_usd: f64,
}

impl InferenceReceipt {
    pub fn new(text: &str, audio_duration_sec: f64, rtt_ms: u64, cache_hit: bool) -> Self {
        let preview: String = text.chars().take(60).collect();
        let chars = text.chars().count();
        // Estimate: cache hit = 0 GPU time. Cold start ~3s + ~0.3s per audio second.
        let gpu_sec = if cache_hit {
            0.0
        } else {
            3.0 + audio_duration_sec * 0.3
        };
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            text_preview: preview,
            text_chars: chars,
            audio_duration_sec,
            rtt_ms,
            cache_hit,
            gpu_sec,
            cost_usd: gpu_sec * MODAL_T4_COST_PER_SEC,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostLedger {
    pub inference: Vec<InferenceReceipt>,
}

impl CostLedger {
    pub fn total_cost(&self) -> f64 {
        self.inference.iter().map(|r| r.cost_usd).sum()
    }

    pub fn call_count(&self) -> usize {
        self.inference.len()
    }

    pub fn cache_hits(&self) -> usize {
        self.inference.iter().filter(|r| r.cache_hit).count()
    }

    pub fn total_audio_sec(&self) -> f64 {
        self.inference.iter().map(|r| r.audio_duration_sec).sum()
    }
}

fn ledger_path() -> PathBuf {
    super::data_dir().join("cost-ledger.json")
}

pub fn load() -> CostLedger {
    let path = ledger_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => CostLedger::default(),
    }
}

pub fn record_inference(receipt: InferenceReceipt) {
    let mut ledger = load();
    ledger.inference.push(receipt);
    let path = ledger_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&ledger).unwrap_or_default(),
    )
    .ok();
}

pub fn print_summary() {
    let ledger = load();
    if ledger.call_count() == 0 {
        println!("No inference calls recorded yet.");
        return;
    }
    let hit_pct = ledger.cache_hits() as f64 / ledger.call_count() as f64 * 100.0;
    println!("  Modal inference spend");
    println!("  ─────────────────────────────────────");
    println!("  Calls:        {}", ledger.call_count());
    println!("  Cache hits:   {} ({:.0}%)", ledger.cache_hits(), hit_pct);
    println!("  Audio synth:  {:.1}s total", ledger.total_audio_sec());
    println!("  Est. cost:    ${:.4}", ledger.total_cost());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ledger_zero_cost() {
        let l = CostLedger::default();
        assert_eq!(l.total_cost(), 0.0);
        assert_eq!(l.call_count(), 0);
    }

    #[test]
    fn cache_hit_costs_nothing() {
        let r = InferenceReceipt::new("привет", 0.5, 10, true);
        assert_eq!(r.cost_usd, 0.0);
        assert_eq!(r.gpu_sec, 0.0);
    }

    #[test]
    fn cache_miss_has_cost() {
        let r = InferenceReceipt::new("привет мир", 3.0, 5500, false);
        assert!(r.cost_usd > 0.0);
        // 3.0 cold start + 3.0 * 0.3 = 3.9s GPU
        assert!((r.gpu_sec - 3.9).abs() < 0.01);
    }

    #[test]
    fn ledger_totals_sum() {
        let l = CostLedger {
            inference: vec![
                InferenceReceipt::new("a", 2.0, 5000, false),
                InferenceReceipt::new("b", 1.0, 10, true),
            ],
        };
        assert_eq!(l.call_count(), 2);
        assert_eq!(l.cache_hits(), 1);
        assert!(l.total_cost() > 0.0);
    }

    #[test]
    fn text_preview_truncated() {
        let long = "а".repeat(200);
        let r = InferenceReceipt::new(&long, 5.0, 4000, false);
        assert_eq!(r.text_preview.chars().count(), 60);
        assert_eq!(r.text_chars, 200);
    }
}
