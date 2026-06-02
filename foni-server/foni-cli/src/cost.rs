//! Persistent cost ledger — tracks every cloud GPU spend.
//!
//! Stored at $XDG_DATA_HOME/foni/cost-ledger.json.
//! Survives reboots. Append-only.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A receipt for one training run — everything you need to know about what happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub timestamp: String,
    pub model_name: String,
    pub action: String,

    // Cloud
    pub gpu: String,
    pub pod_id: String,
    pub provider: String,

    // Time
    pub started_at: String,
    pub finished_at: String,
    pub duration_min: f64,

    // Money
    pub cost_per_hr: f64,
    pub cost_usd: f64,
    pub balance_before: f64,
    pub balance_after: f64,

    // Training
    pub epochs: u32,
    pub final_loss: f64,
    pub dataset_files: usize,
    pub dataset_duration_min: f64,

    // Quality gate
    pub old_mean_gap: f64,
    pub new_mean_gap: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostLedger {
    pub receipts: Vec<Receipt>,
}

impl CostLedger {
    pub fn total_cost(&self) -> f64 {
        self.receipts.iter().map(|r| r.cost_usd).sum()
    }

    pub fn total_gpu_hours(&self) -> f64 {
        self.receipts.iter().map(|r| r.duration_min / 60.0).sum()
    }

    pub fn run_count(&self) -> usize {
        self.receipts.len()
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

pub fn save_receipt(receipt: Receipt) {
    let mut ledger = load();
    ledger.receipts.push(receipt);
    let path = ledger_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let json = serde_json::to_string_pretty(&ledger).unwrap_or_default();
    std::fs::write(&path, json).ok();
}

/// Print a human-readable receipt.
pub fn print_receipt(r: &Receipt) {
    use owo_colors::OwoColorize;

    let rule = "\u{2500}".repeat(52);
    let verdict = if r.passed {
        format!("{:.1}% {}", r.new_mean_gap, "PASS".green().bold())
    } else {
        format!("{:.1}% {}", r.new_mean_gap, "FAIL".red().bold())
    };

    println!(
        "\n  {rule}\n\
         \x20 Training Receipt\n\
         \x20 {rule}\n\
         \x20 Model:         {}\n\
         \x20 Action:        {}\n\
         \x20 GPU:           {}\n\
         \x20 Provider:      {}\n\
         \x20 Duration:      {:.1} min\n\
         \x20 Epochs:        {}\n\
         \x20 Final loss:    {:.6}\n\
         \x20 Dataset:       {} files ({:.1} min)\n\
         \n\
         \x20 Cost/hr:       ${:.2}\n\
         \x20 Total cost:    {}\n\
         \x20 Balance:       ${:.2} \u{2192} ${:.2}\n\
         \n\
         \x20 Quality gate:  {:.1}% \u{2192} {}\n\
         \x20 {rule}",
        r.model_name.bold(),
        r.action,
        r.gpu,
        r.provider,
        r.duration_min,
        r.epochs,
        r.final_loss,
        r.dataset_files,
        r.dataset_duration_min,
        r.cost_per_hr,
        format!("${:.2}", r.cost_usd).yellow(),
        r.balance_before,
        r.balance_after,
        r.old_mean_gap,
        verdict,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_receipt(cost: f64) -> Receipt {
        Receipt {
            timestamp: "2026-06-02T00:00:00Z".into(),
            model_name: "sidorovich".into(),
            action: "train".into(),
            gpu: "RTX 3090".into(),
            pod_id: "test-pod".into(),
            provider: "RunPod".into(),
            started_at: "2026-06-02T00:00:00Z".into(),
            finished_at: "2026-06-02T02:00:00Z".into(),
            duration_min: 120.0,
            cost_per_hr: 0.22,
            cost_usd: cost,
            balance_before: 10.0,
            balance_after: 10.0 - cost,
            epochs: 500,
            final_loss: 0.003,
            dataset_files: 189,
            dataset_duration_min: 27.7,
            old_mean_gap: 39.5,
            new_mean_gap: 28.0,
            passed: true,
        }
    }

    #[test]
    fn empty_ledger_has_zero_total() {
        let l = CostLedger::default();
        assert_eq!(l.total_cost(), 0.0);
        assert_eq!(l.run_count(), 0);
    }

    #[test]
    fn total_sums_receipts() {
        let l = CostLedger {
            receipts: vec![test_receipt(0.44), test_receipt(0.66)],
        };
        assert!((l.total_cost() - 1.10).abs() < 0.01);
        assert_eq!(l.run_count(), 2);
        assert!(l.total_gpu_hours() > 3.9);
    }

    #[test]
    fn receipt_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foni/cost-ledger.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let mut ledger = CostLedger::default();
        ledger.receipts.push(test_receipt(0.22));
        std::fs::write(&path, serde_json::to_string_pretty(&ledger).unwrap()).unwrap();

        let loaded: CostLedger =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.run_count(), 1);
        assert!((loaded.total_cost() - 0.22).abs() < 0.01);
        assert_eq!(loaded.receipts[0].pod_id, "test-pod");
        assert_eq!(loaded.receipts[0].epochs, 500);
        assert!(loaded.receipts[0].passed);
    }
}
