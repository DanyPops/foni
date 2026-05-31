/// Rule-based plain-text → SSML annotator for Russian espeak-ng.
///
/// Mirrors pipeline/prosody.ts break values exactly so Rust synthesis
/// produces identical pause structure to the TypeScript path.
/// Output is a full <speak>...</speak> document for `espeak-ng -m`.

const BREAK_COMMA_MS: u32 = 150;
const BREAK_SEMICOLON_MS: u32 = 220;
const BREAK_COLON_MS: u32 = 200;
const BREAK_DASH_MS: u32 = 180;
const BREAK_ELLIPSIS_MS: u32 = 420;
const BREAK_PERIOD_MS: u32 = 320;
const BREAK_EXCLAIM_MS: u32 = 280;
const BREAK_QUESTION_MS: u32 = 300;

fn brk(ms: u32) -> String {
    format!(r#"<break time="{}ms"/>"#, ms)
}

/// Inject SSML break tags at punctuation boundaries.
/// Returns a `<speak>...</speak>` document suitable for `espeak-ng -v ru -m`.
pub fn annotate(text: &str) -> String {
    let annotated = annotate_punctuation(text);
    format!("<speak>{}</speak>", annotated)
}

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
}
