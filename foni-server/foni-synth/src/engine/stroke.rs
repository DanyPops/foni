//! Stroke splitter — breaks text into atomic synthesis units (clauses).
//!
//! A stroke is the smallest unit where expression can change.
//! Delimited by punctuation that implies a breath or pause.

/// A single synthesis stroke — one clause, one shade.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub text: String,
    pub delimiter: Delimiter,
}

/// What ended this stroke — determines the pause between strokes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Delimiter {
    Comma,
    Semicolon,
    Colon,
    Dash,
    Ellipsis,
    Period,
    Exclamation,
    Question,
    Newline,
    End,
}

/// Split text into strokes at clause boundaries.
pub fn split_strokes(text: &str) -> Vec<Stroke> {
    let mut strokes = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        match ch {
            '.' => {
                if i + 2 < len && chars[i + 1] == '.' && chars[i + 2] == '.' {
                    push_stroke(&mut strokes, &mut current, Delimiter::Ellipsis);
                    i += 3;
                    continue;
                }
                if i + 1 < len && chars[i + 1] == '.' {
                    push_stroke(&mut strokes, &mut current, Delimiter::Ellipsis);
                    i += 2;
                    continue;
                }
                push_stroke(&mut strokes, &mut current, Delimiter::Period);
            }
            '!' | '！' => push_stroke(&mut strokes, &mut current, Delimiter::Exclamation),
            '?' | '？' => push_stroke(&mut strokes, &mut current, Delimiter::Question),
            ',' | '，' => push_stroke(&mut strokes, &mut current, Delimiter::Comma),
            ';' => push_stroke(&mut strokes, &mut current, Delimiter::Semicolon),
            ':' => push_stroke(&mut strokes, &mut current, Delimiter::Colon),
            '—' | '–' => push_stroke(&mut strokes, &mut current, Delimiter::Dash),
            '\n' => {
                if !current.trim().is_empty() {
                    push_stroke(&mut strokes, &mut current, Delimiter::Newline);
                }
            }
            '…' => push_stroke(&mut strokes, &mut current, Delimiter::Ellipsis),
            _ => current.push(ch),
        }
        i += 1;
    }

    if !current.trim().is_empty() {
        push_stroke(&mut strokes, &mut current, Delimiter::End);
    }

    strokes
}

fn push_stroke(strokes: &mut Vec<Stroke>, current: &mut String, delimiter: Delimiter) {
    let text = current.trim().to_string();
    if !text.is_empty() {
        strokes.push(Stroke { text, delimiter });
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_sentence() {
        let strokes = split_strokes("The Emperor protects.");
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].text, "The Emperor protects");
        assert_eq!(strokes[0].delimiter, Delimiter::Period);
    }

    #[test]
    fn comma_splits() {
        let strokes = split_strokes("Brother, rise now.");
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].text, "Brother");
        assert_eq!(strokes[0].delimiter, Delimiter::Comma);
        assert_eq!(strokes[1].text, "rise now");
        assert_eq!(strokes[1].delimiter, Delimiter::Period);
    }

    #[test]
    fn dash_splits() {
        let strokes = split_strokes("Rise — for the Emperor!");
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].text, "Rise");
        assert_eq!(strokes[0].delimiter, Delimiter::Dash);
        assert_eq!(strokes[1].text, "for the Emperor");
        assert_eq!(strokes[1].delimiter, Delimiter::Exclamation);
    }

    #[test]
    fn exclamation_and_question() {
        let strokes = split_strokes("Charge! Do you hear me?");
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].delimiter, Delimiter::Exclamation);
        assert_eq!(strokes[1].delimiter, Delimiter::Question);
    }

    #[test]
    fn ellipsis_three_dots() {
        let strokes = split_strokes("The warp... it calls.");
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].delimiter, Delimiter::Ellipsis);
    }

    #[test]
    fn unicode_ellipsis() {
        let strokes = split_strokes("The warp… it calls.");
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].delimiter, Delimiter::Ellipsis);
    }

    #[test]
    fn semicolon_splits() {
        let strokes = split_strokes("We fight; we die; we prevail.");
        assert_eq!(strokes.len(), 3);
        assert_eq!(strokes[0].delimiter, Delimiter::Semicolon);
        assert_eq!(strokes[1].delimiter, Delimiter::Semicolon);
        assert_eq!(strokes[2].delimiter, Delimiter::Period);
    }

    #[test]
    fn empty_string() {
        assert!(split_strokes("").is_empty());
    }

    #[test]
    fn whitespace_only() {
        assert!(split_strokes("   \n  ").is_empty());
    }

    #[test]
    fn no_punctuation() {
        let strokes = split_strokes("The Emperor protects");
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].delimiter, Delimiter::End);
    }

    #[test]
    fn multiple_sentences() {
        let strokes = split_strokes("Stand firm. Hold the line! For the Emperor?");
        assert_eq!(strokes.len(), 3);
    }

    #[test]
    fn newline_splits() {
        let strokes = split_strokes("First line\nSecond line");
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].delimiter, Delimiter::Newline);
    }

    #[test]
    fn complex_diomedes_line() {
        let strokes = split_strokes(
            "Julian, hear me — the Imperium's will is absolute! \
             We shall write... and the galaxy will tremble.",
        );
        assert_eq!(strokes.len(), 5);
        assert_eq!(strokes[0].text, "Julian");
        assert_eq!(strokes[0].delimiter, Delimiter::Comma);
        assert_eq!(strokes[1].text, "hear me");
        assert_eq!(strokes[1].delimiter, Delimiter::Dash);
    }

    #[test]
    fn russian_text() {
        let strokes = split_strokes("Подойди-ка, надо тебе ситуацию прояснить.");
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].delimiter, Delimiter::Comma);
    }

    #[test]
    fn colon_splits() {
        let strokes = split_strokes("Remember this: the Emperor protects.");
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].delimiter, Delimiter::Colon);
    }
}
