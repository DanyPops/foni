use std::io::Write;
/// Word Error Rate via Whisper CLI transcription.
///
/// WER = (S + D + I) / N  where S=substitutions, D=deletions, I=insertions, N=ref words.
/// Implemented with Wagner-Fischer dynamic programming (pure Rust).
///
/// Invokes the `whisper` binary (openai-whisper CLI) — no inline Python.
/// Results are returned as `None` when Whisper is unavailable (test gating).
use std::process::Command;
use tempfile::{NamedTempFile, TempDir};

#[derive(Debug, Clone)]
pub struct WerResult {
    pub transcript: String,
    pub wer_pct: f32,
    pub edits: u32,
    pub ref_words: u32,
}

/// Transcribe a WAV file using Whisper and compute WER against `reference`.
/// Returns `None` if Whisper is not installed or fails.
pub fn compute_wer(wav_bytes: &[u8], reference: &str, language: &str) -> Option<WerResult> {
    // Write WAV to a temp file (Whisper needs a file path)
    let mut tmp = NamedTempFile::with_suffix(".wav").ok()?;
    tmp.write_all(wav_bytes).ok()?;
    let tmp_path = tmp.path().to_str()?.to_string();

    // Run: whisper --model base --language <lang> --output_format txt <file>
    // The whisper CLI writes <stem>.txt in the output directory.
    let out_dir = TempDir::new().ok()?;
    let out = Command::new("whisper")
        .args([
            tmp_path.as_str(),
            "--model",
            "base",
            "--language",
            language,
            "--output_format",
            "txt",
            "--output_dir",
            out_dir.path().to_str()?,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }

    // Read the generated .txt file
    let txt_file = out_dir
        .path()
        .join(
            std::path::Path::new(&tmp_path)
                .file_stem()?
                .to_string_lossy()
                .as_ref(),
        )
        .with_extension("txt");
    let transcript = std::fs::read_to_string(&txt_file).ok()?.trim().to_string();
    if transcript.is_empty() {
        return None;
    }

    let (edits, ref_words) = edit_distance_words(reference, &transcript);
    let wer_pct = if ref_words == 0 {
        0.0
    } else {
        edits as f32 / ref_words as f32 * 100.0
    };

    Some(WerResult {
        transcript,
        wer_pct,
        edits,
        ref_words,
    })
}

/// Wagner-Fischer edit distance on whitespace-tokenised words.
/// Returns (edits, reference_word_count).
pub fn edit_distance_words(reference: &str, hypothesis: &str) -> (u32, u32) {
    // Normalise: lowercase, strip punctuation
    let norm = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .map(String::from)
            .collect()
    };

    let r = norm(reference);
    let h = norm(hypothesis);
    let n = r.len();
    let m = h.len();

    if n == 0 {
        return (m as u32, 0);
    }

    // DP table
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 0..=n {
        dp[i][0] = i as u32;
    }
    for j in 0..=m {
        dp[0][j] = j as u32;
    }
    for i in 1..=n {
        for j in 1..=m {
            dp[i][j] = if r[i - 1] == h[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
            };
        }
    }
    (dp[n][m], n as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_transcripts_zero_wer() {
        let (edits, n) = edit_distance_words("подойди ка надо тебе", "подойди ка надо тебе");
        assert_eq!(edits, 0);
        assert_eq!(n, 4);
    }

    #[test]
    fn one_substitution() {
        let (edits, n) = edit_distance_words("подойди ка надо тебе", "подойди ка здесь тебе");
        assert_eq!(edits, 1);
        assert_eq!(n, 4);
    }

    #[test]
    fn punctuation_stripped() {
        let (edits, n) = edit_distance_words("Подойди-ка, надо.", "подойди ка надо");
        assert_eq!(edits, 0, "punctuation and case should be ignored");
        let _ = n;
    }

    #[test]
    fn empty_hypothesis_all_deletions() {
        let (edits, n) = edit_distance_words("раз два три", "");
        assert_eq!(edits, n); // all 3 words deleted
    }
}
