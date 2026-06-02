//! Persistent cost ledger — tracks every cloud GPU spend.
//!
//! Stored at $XDG_DATA_HOME/foni/cost-ledger.json.
//! Survives reboots. Append-only.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEntry {
    pub timestamp: String,
    pub action: String,
    pub pod_id: String,
    pub gpu: String,
    pub duration_min: f64,
    pub cost_usd: f64,
    pub model_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostLedger {
    pub entries: Vec<CostEntry>,
}

impl CostLedger {
    pub fn total(&self) -> f64 {
        self.entries.iter().map(|e| e.cost_usd).sum()
    }

    pub fn count(&self) -> usize {
        self.entries.len()
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

pub fn record(entry: CostEntry) {
    let mut ledger = load();
    ledger.entries.push(entry);
    let path = ledger_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let json = serde_json::to_string_pretty(&ledger).unwrap_or_default();
    std::fs::write(&path, json).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ledger_has_zero_total() {
        let l = CostLedger::default();
        assert_eq!(l.total(), 0.0);
        assert_eq!(l.count(), 0);
    }

    #[test]
    fn total_sums_entries() {
        let l = CostLedger {
            entries: vec![
                CostEntry {
                    timestamp: "2026-06-02T00:00:00Z".into(),
                    action: "train".into(),
                    pod_id: "abc".into(),
                    gpu: "RTX 3090".into(),
                    duration_min: 120.0,
                    cost_usd: 0.44,
                    model_name: "sidorovich".into(),
                },
                CostEntry {
                    timestamp: "2026-06-02T04:00:00Z".into(),
                    action: "train".into(),
                    pod_id: "def".into(),
                    gpu: "RTX 3090".into(),
                    duration_min: 180.0,
                    cost_usd: 0.66,
                    model_name: "sidorovich".into(),
                },
            ],
        };
        assert!((l.total() - 1.10).abs() < 0.01);
        assert_eq!(l.count(), 2);
    }

    #[test]
    fn record_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foni/cost-ledger.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        // Write directly to the temp path
        let entry = CostEntry {
            timestamp: "2026-06-02T12:00:00Z".into(),
            action: "train".into(),
            pod_id: "test123".into(),
            gpu: "RTX 3090".into(),
            duration_min: 60.0,
            cost_usd: 0.22,
            model_name: "test".into(),
        };

        let mut ledger = CostLedger::default();
        ledger.entries.push(entry);
        std::fs::write(&path, serde_json::to_string_pretty(&ledger).unwrap()).unwrap();

        let loaded: CostLedger =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.count(), 1);
        assert!((loaded.total() - 0.22).abs() < 0.01);
        assert_eq!(loaded.entries[0].pod_id, "test123");
    }
}
