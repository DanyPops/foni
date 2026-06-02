use crate::gap::{GapResult, GapRow};
use tabled::{
    settings::{object::Columns, Alignment, Modify, Style},
    Table, Tabled,
};

#[derive(Tabled)]
struct GapDisplay {
    #[tabled(rename = "Metric")]
    metric: String,
    #[tabled(rename = "Target")]
    target: String,
    #[tabled(rename = "Actual")]
    actual: String,
    #[tabled(rename = "Gap%")]
    gap_pct: String,
    #[tabled(rename = "Verdict")]
    verdict: String,
}

impl From<&GapRow> for GapDisplay {
    fn from(r: &GapRow) -> Self {
        GapDisplay {
            metric: r.metric.clone(),
            target: r.target.clone(),
            actual: r.actual.clone(),
            gap_pct: format!("{:.1}%", r.gap_pct),
            verdict: r.verdict.label().to_string(),
        }
    }
}

pub fn format_gap_table(r: &GapResult) -> String {
    let rows: Vec<GapDisplay> = r.rows.iter().map(GapDisplay::from).collect();
    let table = Table::new(&rows)
        .with(Style::rounded())
        .with(Modify::new(Columns::new(3..4)).with(Alignment::right()))
        .to_string();
    format!(
        "Phrase: \"{}\"\n{}\nMean gap: {:.1}%",
        r.phrase, table, r.mean_gap_pct
    )
}

pub fn format_gap_summary(results: &[GapResult]) -> String {
    #[derive(Tabled)]
    struct SummaryRow {
        #[tabled(rename = "Phrase")]
        phrase: String,
        #[tabled(rename = "Gap%")]
        gap: String,
    }
    let rows: Vec<SummaryRow> = results
        .iter()
        .map(|r| SummaryRow {
            phrase: r.phrase.clone(),
            gap: format!("{:.1}%", r.mean_gap_pct),
        })
        .collect();
    let mean = results.iter().map(|r| r.mean_gap_pct).sum::<f32>() / results.len().max(1) as f32;
    let table = Table::new(&rows).with(Style::rounded()).to_string();
    format!("{table}\nMean gap: {mean:.1}%")
}
