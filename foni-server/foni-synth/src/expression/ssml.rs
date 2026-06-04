// Rule-based plain-text → SSML prosody annotator for Russian espeak-ng.
// Ports pipeline/prosody.ts: deterministic per-sentence rate/pitch/range variation.
use crate::config::BreaksConfig;

pub const BREAK_COMMA_MS: u32 = 150;
pub const BREAK_SEMICOLON_MS: u32 = 220;
pub const BREAK_COLON_MS: u32 = 180;
pub const BREAK_DASH_MS: u32 = 200;
pub const BREAK_ELLIPSIS_MS: u32 = 420;
pub const BREAK_PERIOD_MS: u32 = 320;
pub const BREAK_EXCLAIM_MS: u32 = 300;
pub const BREAK_QUESTION_MS: u32 = 350;

impl BreaksConfig {
    pub fn comma(&self) -> u32 {
        self.comma
    }
    pub fn semicolon(&self) -> u32 {
        self.semicolon
    }
    pub fn colon(&self) -> u32 {
        self.colon
    }
    pub fn dash(&self) -> u32 {
        self.dash
    }
    pub fn ellipsis(&self) -> u32 {
        self.ellipsis
    }
    pub fn period(&self) -> u32 {
        self.period
    }
    pub fn exclamation(&self) -> u32 {
        self.exclamation
    }
    pub fn question(&self) -> u32 {
        self.question
    }
}

fn brk(ms: u32) -> String {
    format!(r#"<break time="{}ms"/>"#, ms)
}

/// Inject SSML break tags at punctuation boundaries.
/// Returns a `<speak>...</speak>` document suitable for `espeak-ng -v ru -m`.
pub fn annotate(text: &str) -> String {
    let annotated = annotate_punctuation(text);
    format!("<speak>{}</speak>", annotated)
}

// ─── Prosody variation ───────────────────────────────────────────────────────

/// Per-sentence rate/range — matches pipeline/prosody.ts sentenceProsody().
/// pitch is omitted: bare integer pitch= values in espeak SSML shift the
/// fundamental frequency into the wrong range (~3× natural). range= is
/// sufficient for expressiveness; absolute pitch is set by espeak voice choice.
#[allow(dead_code)]
pub struct SentenceProsody {
    pub rate: i32,           // % of baseline (100 = normal)
    pub range: &'static str, // "x-high" | "high" | "medium"
}

const RATE_JITTER_PCT: i32 = 6;
const PHRASE_FINAL_REDUCTION: i32 = 8;

/// FNV-1a hash → 0.0..1.0. Same algorithm as hashStr() in prosody.ts.
#[allow(dead_code)]
fn hash01(s: &str) -> f64 {
    let mut h: u32 = 2_166_136_261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    h as f64 / 0xFFFF_FFFF_u32 as f64
}

/// Compute deterministic prosody for a sentence.
#[allow(dead_code)]
pub fn sentence_prosody(sentence: &str) -> SentenceProsody {
    let rng = hash01(sentence);
    let rate = 100 + ((rng - 0.5) * 2.0 * RATE_JITTER_PCT as f64).round() as i32;
    let is_q = sentence.trim_end().ends_with('?');
    let is_ex = sentence.trim_end().ends_with('!');

    if is_q {
        SentenceProsody {
            rate,
            range: "high",
        }
    } else if is_ex {
        SentenceProsody {
            rate: rate + 3,
            range: "x-high",
        }
    } else {
        SentenceProsody {
            rate,
            range: "medium",
        }
    }
}

/// Build a full `<speak>…</speak>` document with per-sentence prosody.
/// Each sentence is wrapped in `<prosody rate pitch range>` and gets break-
/// tagged punctuation. The last clause of each sentence gets phrase-final
/// rate slowing (mirrors ProsodyBackend in prosody.ts).
#[allow(dead_code)]
pub fn annotate_with_prosody(text: &str) -> String {
    // Split on sentence boundaries: . ! ?
    let mut body = String::with_capacity(text.len() * 3);
    let mut sentence_start = 0;

    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let is_sentence_end = matches!(c, '.' | '!' | '?')
            && !(c == '.'
                && i > 0
                && chars[i - 1].is_ascii_digit()
                && chars
                    .get(i + 1)
                    .map(|x| x.is_ascii_digit())
                    .unwrap_or(false));

        if is_sentence_end {
            let sentence: String = chars[sentence_start..=i].iter().collect();
            let sentence = sentence.trim();
            if !sentence.is_empty() {
                let p = sentence_prosody(sentence);
                let inner = annotate_punctuation(sentence);
                // Phrase-final slowing on the last part after the final comma
                let slowed = apply_phrase_final_slowing(&inner);
                body.push_str(&format!(
                    r#"<prosody rate="{rate}%" range="{range}">{slowed}</prosody> "#,
                    rate = p.rate,
                    range = p.range,
                ));
            }
            sentence_start = i + 1;
        }
        i += 1;
    }

    // Any remaining text (unterminated sentence)
    let tail: String = chars[sentence_start..].iter().collect();
    let tail = tail.trim();
    if !tail.is_empty() {
        let p = sentence_prosody(tail);
        let inner = annotate_punctuation(tail);
        body.push_str(&format!(
            r#"<prosody rate="{rate}%" range="{range}">{inner}</prosody>"#,
            rate = p.rate,
            range = p.range,
        ));
    }

    format!("<speak>{}</speak>", body.trim())
}

/// Wrap the text after the last comma in phrase-final rate slowing.
#[allow(dead_code)]
fn apply_phrase_final_slowing(text: &str) -> String {
    // Find the last comma-break tag boundary
    let marker = r#"<break time="150ms"/>"#; // BREAK_COMMA_MS
    if let Some(pos) = text.rfind(marker) {
        let split = pos + marker.len();
        let before = &text[..split];
        let after = text[split..].trim_start();
        let rate = 100 - PHRASE_FINAL_REDUCTION;
        format!(r#"{before}<prosody rate="{rate}%">{after}</prosody>"#)
    } else {
        text.to_string()
    }
}

// ─── Punctuation annotation ───────────────────────────────────────────────────

/// Inject break tags inline — does not wrap in <speak>.
pub fn annotate_punctuation(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Ellipsis (…  or ...)
        if c == '…' {
            out.push(c);
            out.push_str(&brk(BREAK_ELLIPSIS_MS));
            i += 1;
            continue;
        }
        if c == '.' && chars.get(i + 1) == Some(&'.') && chars.get(i + 2) == Some(&'.') {
            out.push_str("...");
            out.push_str(&brk(BREAK_ELLIPSIS_MS));
            i += 3;
            continue;
        }

        // Sentence-ending punctuation
        if c == '.' {
            // Don't break mid-number (3.14) — check neighbours
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = chars
                .get(i + 1)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            out.push(c);
            if !prev_digit && !next_digit {
                out.push_str(&brk(BREAK_PERIOD_MS));
            }
            i += 1;
            continue;
        }
        if c == '!' {
            out.push(c);
            out.push_str(&brk(BREAK_EXCLAIM_MS));
            i += 1;
            continue;
        }
        if c == '?' {
            out.push(c);
            out.push_str(&brk(BREAK_QUESTION_MS));
            i += 1;
            continue;
        }

        // Interior punctuation
        if c == ',' {
            out.push(c);
            out.push(' ');
            out.push_str(&brk(BREAK_COMMA_MS));
            // Consume trailing space already added
            if chars.get(i + 1) == Some(&' ') {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == ';' {
            out.push(c);
            out.push(' ');
            out.push_str(&brk(BREAK_SEMICOLON_MS));
            if chars.get(i + 1) == Some(&' ') {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == ':' {
            // Skip colons in numbers (12:30)
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = chars
                .get(i + 1)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            out.push(c);
            if !prev_digit && !next_digit {
                out.push(' ');
                out.push_str(&brk(BREAK_COLON_MS));
            }
            if !prev_digit && !next_digit && chars.get(i + 1) == Some(&' ') {
                i += 1;
            }
            i += 1;
            continue;
        }
        // Em/en dash
        if c == '—' || c == '–' {
            out.push(' ');
            out.push_str(&brk(BREAK_DASH_MS));
            out.push(' ');
            // Consume surrounding spaces
            if chars.get(i + 1) == Some(&' ') {
                i += 1;
            }
            i += 1;
            continue;
        }

        out.push(c);
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comma_gets_break_tag() {
        let out = annotate_punctuation("Подойди-ка, надо");
        assert!(out.contains(r#"<break time="150ms"/>"#), "got: {out}");
    }

    #[test]
    fn period_gets_break_tag() {
        let out = annotate_punctuation("Понял.");
        assert!(out.contains(r#"<break time="320ms"/>"#), "got: {out}");
    }

    #[test]
    fn decimal_number_no_break() {
        let out = annotate_punctuation("3.14 секунды");
        assert!(!out.contains("<break"), "break inserted mid-number: {out}");
    }

    #[test]
    fn annotate_wraps_in_speak() {
        let out = annotate("Понял.");
        assert!(out.starts_with("<speak>"));
        assert!(out.ends_with("</speak>"));
    }

    #[test]
    fn full_phrase_has_comma_and_period_breaks() {
        let out = annotate_punctuation("Подойди-ка, надо тебе ситуацию прояснить.");
        assert!(
            out.contains(r#"<break time="150ms"/>"#),
            "missing comma break"
        );
        assert!(
            out.contains(r#"<break time="320ms"/>"#),
            "missing period break"
        );
    }

    #[test]
    fn prosody_wraps_each_sentence() {
        let out = annotate_with_prosody("Подойди-ка, надо тебе ситуацию прояснить.");
        assert!(out.starts_with("<speak>"), "missing <speak>");
        assert!(out.ends_with("</speak>"), "missing </speak>");
        assert!(out.contains("<prosody rate="), "missing prosody wrapper");
    }

    #[test]
    fn prosody_deterministic_for_same_text() {
        let a = annotate_with_prosody("Понял.");
        let b = annotate_with_prosody("Понял.");
        assert_eq!(a, b, "prosody must be deterministic");
    }

    #[test]
    fn question_gets_high_range() {
        let out = annotate_with_prosody("Как дела?");
        assert!(
            out.contains(r#"range="high""#),
            "question should have high range: {out}"
        );
    }

    #[test]
    fn hash01_stable() {
        let h1 = hash01("Понял.");
        let h2 = hash01("Понял.");
        assert!((h1 - h2).abs() < 1e-10);
        assert!(h1 > 0.0 && h1 < 1.0);
    }
}
