//! F0 and energy contour correlation via DTW + Pearson.
//! Catches flat-robotic synthesis that has the right F0 mean but wrong shape.

/// Naive O(n²) DTW distance between two sequences.
/// Bounded to max 1000 frames each (10s of speech at 10ms hop) — fast enough.
fn dtw_path(a: &[f32], b: &[f32]) -> Vec<(usize, usize)> {
    let n = a.len().min(1000);
    let m = b.len().min(1000);
    if n == 0 || m == 0 {
        return vec![];
    }

    let a = &a[..n];
    let b = &b[..m];

    // Cost matrix (filled forward)
    let inf = f32::INFINITY;
    let mut cost = vec![vec![inf; m]; n];
    cost[0][0] = (a[0] - b[0]).abs();
    for i in 1..n {
        cost[i][0] = cost[i - 1][0] + (a[i] - b[0]).abs();
    }
    for j in 1..m {
        cost[0][j] = cost[0][j - 1] + (a[0] - b[j]).abs();
    }
    for i in 1..n {
        for j in 1..m {
            let prev = cost[i - 1][j].min(cost[i][j - 1]).min(cost[i - 1][j - 1]);
            cost[i][j] = prev + (a[i] - b[j]).abs();
        }
    }

    // Backtrack
    let mut path = Vec::new();
    let (mut i, mut j) = (n - 1, m - 1);
    path.push((i, j));
    while i > 0 || j > 0 {
        if i == 0 {
            j -= 1;
        } else if j == 0 {
            i -= 1;
        } else {
            let best = cost[i - 1][j - 1].min(cost[i - 1][j]).min(cost[i][j - 1]);
            if best == cost[i - 1][j - 1] {
                i -= 1;
                j -= 1;
            } else if best == cost[i - 1][j] {
                i -= 1;
            } else {
                j -= 1;
            }
        }
        path.push((i, j));
    }
    path.reverse();
    path
}

/// Pearson correlation of two equal-length slices. Returns 0.0 if degenerate.
fn pearson(x: &[f32], y: &[f32]) -> f32 {
    assert_eq!(x.len(), y.len());
    let n = x.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mx = x.iter().map(|&v| v as f64).sum::<f64>() / n;
    let my = y.iter().map(|&v| v as f64).sum::<f64>() / n;
    let num: f64 = x
        .iter()
        .zip(y)
        .map(|(&a, &b)| (a as f64 - mx) * (b as f64 - my))
        .sum();
    let da: f64 = x
        .iter()
        .map(|&a| (a as f64 - mx).powi(2))
        .sum::<f64>()
        .sqrt();
    let db: f64 = y
        .iter()
        .map(|&b| (b as f64 - my).powi(2))
        .sum::<f64>()
        .sqrt();
    if da * db < 1e-12 {
        return 0.0;
    }
    (num / (da * db)).clamp(-1.0, 1.0) as f32
}

/// DTW-align two contours and compute Pearson correlation of the aligned pairs.
pub fn contour_correlation(reference: &[f32], synthesis: &[f32]) -> f32 {
    if reference.is_empty() || synthesis.is_empty() {
        return 0.0;
    }
    let path = dtw_path(reference, synthesis);
    if path.is_empty() {
        return 0.0;
    }
    let aligned_ref: Vec<f32> = path.iter().map(|&(i, _)| reference[i]).collect();
    let aligned_syn: Vec<f32> = path.iter().map(|&(_, j)| synthesis[j]).collect();
    pearson(&aligned_ref, &aligned_syn)
}

/// Compute F0 contour correlation and energy envelope correlation in one pass.
/// Returns (f0_corr, energy_corr) ∈ [−1, 1].
pub fn compute_contour_correlations(
    ref_f0: &[f32],
    ref_energy: &[f32],
    syn_f0: &[f32],
    syn_energy: &[f32],
) -> (f32, f32) {
    (
        contour_correlation(ref_f0, syn_f0),
        contour_correlation(ref_energy, syn_energy),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine_contour(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * i as f32 / n as f32).sin())
            .collect()
    }

    #[test]
    fn identical_contours_correlate_perfectly() {
        let c = sine_contour(100);
        let r = contour_correlation(&c, &c);
        assert!((r - 1.0).abs() < 0.01, "r={r}");
    }

    #[test]
    fn flat_vs_varying_means_low_score() {
        // Flat contour (monotone robot) vs sine (natural variation)
        // DTW can align them but Pearson on aligned pairs still near zero
        // because one sequence has no variation.
        let varying = sine_contour(100);
        let flat = vec![0.0f32; 100];
        let r = contour_correlation(&varying, &flat);
        // Flat sequence has zero variance → Pearson undefined → returns 0.0
        assert_eq!(r, 0.0, "flat contour should give 0.0 correlation");
    }

    #[test]
    fn flat_vs_varying_low_correlation() {
        let varying = sine_contour(200);
        let flat = vec![0.5f32; 200];
        let r = contour_correlation(&varying, &flat);
        assert!(r.abs() < 0.2, "r={r}");
    }

    #[test]
    fn different_length_handled() {
        let a = sine_contour(100);
        let b = sine_contour(150);
        let r = contour_correlation(&a, &b);
        assert!(r > 0.9, "same shape, different length: r={r}");
    }
}
