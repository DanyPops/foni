use crate::gap::{GapResult, GapRow};

const WIDTH: usize = 66;
const SEP:   &str  = "──────────────────────────────────────────────────────────────────";

fn row_line(r: &GapRow) -> String {
    format!(
        "{:<18} {:<14} {:<14} {:>4}%  {}",
        r.metric, r.target, r.actual, r.gap_pct, r.verdict.label()
    )
}

/// Single-phrase table — identical layout to the TypeScript version.
pub fn format_gap_table(r: &GapResult) -> String {
    let header = format!("Phrase: \"{}\"", r.phrase);
    let col_hdr = format!(
        "{:<18} {:<14} {:<14} {:>5}   {}",
        "Metric", "Target", "Actual", "Gap%", "Verdict"
    );
    let mut lines = vec![header, col_hdr, SEP.to_string()];
    for row in &r.rows {
        lines.push(row_line(row));
    }
    lines.push(SEP.to_string());
    lines.push(format!("Mean gap: {}%", r.mean_gap_pct));
    lines.join("\n")
}

/// Multi-phrase summary — identical layout to the TypeScript version.
pub fn format_gap_summary(results: &[GapResult]) -> String {
    let bar = "═".repeat(WIDTH);
    let mut lines = vec![
        bar.clone(),
        "  BASELINE GAP SUMMARY".to_string(),
        SEP.to_string(),
    ];
    for r in results {
        lines.push(format!("  \"{}\"  →  {}%", r.phrase, r.mean_gap_pct));
    }
    let mean = results.iter().map(|r| r.mean_gap_pct).sum::<f32>() / results.len() as f32;
    lines.push(SEP.to_string());
    lines.push(format!("  Mean gap: {:.1}%", mean));
    lines.push(bar);
    lines.join("\n")
}
