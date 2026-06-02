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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gap::Verdict;

    fn dummy_gap_result() -> GapResult {
        GapResult {
            phrase: "test phrase".to_string(),
            rows: vec![
                GapRow {
                    metric: "Loudness".to_string(),
                    target: "-13.5 dBFS".to_string(),
                    actual: "-19.1 dBFS".to_string(),
                    gap_pct: 56.0,
                    verdict: Verdict::Far,
                },
                GapRow {
                    metric: "Brightness".to_string(),
                    target: "2288 Hz".to_string(),
                    actual: "3302 Hz".to_string(),
                    gap_pct: 30.2,
                    verdict: Verdict::Near,
                },
            ],
            mean_gap_pct: 43.1,
        }
    }

    #[test]
    fn format_gap_table_contains_phrase() {
        let table = format_gap_table(&dummy_gap_result());
        assert!(table.contains("test phrase"));
    }

    #[test]
    fn format_gap_table_contains_metrics() {
        let table = format_gap_table(&dummy_gap_result());
        assert!(table.contains("Loudness"));
        assert!(table.contains("Brightness"));
    }

    #[test]
    fn format_gap_table_contains_mean() {
        let table = format_gap_table(&dummy_gap_result());
        assert!(table.contains("43.1%"));
    }

    #[test]
    fn format_gap_summary_with_multiple_results() {
        let results = vec![dummy_gap_result(), dummy_gap_result()];
        let summary = format_gap_summary(&results);
        assert!(summary.contains("test phrase"));
        assert!(summary.contains("43.1%"));
    }

    #[test]
    fn format_gap_summary_empty_input() {
        let summary = format_gap_summary(&[]);
        assert!(summary.contains("0.0%"));
    }
}
