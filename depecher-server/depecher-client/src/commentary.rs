//! Ollama-driven character commentary — shared between depecherd and depecher-cli.
//!
//! `ollama_commentary` calls Ollama's chat API with a character seed and returns
//! a single contextual injection (`Placement` + text). Pure functions
//! `parse_commentary_response` and `apply_commentary` are format-free and testable
//! without a network.

use std::time::Duration;

const SYSTEM_PROMPT: &str = "\
You are roleplaying a character. Output ONE short Russian expletive or \
interjection (1–3 words) and where it goes. \
Do NOT repeat or paraphrase the sentence. Output only the injection.\n\
\n\
Exactly one line, format:\n\
  SUFFIX: блядь\n\
  PREFIX: Ёпта,\n\
  MID: нихуя себе\n\
\n\
One line. Injection only. No sentence. No explanation.";

/// Where to insert the injection relative to the sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    Prefix,
    Mid,
    Suffix,
}

impl std::fmt::Display for Placement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prefix => write!(f, "PREFIX"),
            Self::Mid => write!(f, "MID"),
            Self::Suffix => write!(f, "SUFFIX"),
        }
    }
}

/// A commentary injection ready to apply.
#[derive(Debug, Clone)]
pub struct CommentaryResult {
    pub text: String,
    pub placement: Placement,
}

/// A minimal character seed passed to the LLM as context.
#[derive(Debug, Clone)]
pub struct CharacterSeed<'a> {
    pub persona: &'a str,
    pub expressions: &'a [&'a str],
}

/// Call Ollama to generate one contextual injection for `sentence`.
///
/// Returns `Err` on timeout, HTTP failure, or unparseable response.
/// Callers should fall back to static injection on error.
pub async fn ollama_commentary(
    sentence: &str,
    emotion: &str,
    seed: &CharacterSeed<'_>,
    url: &str,
    model: &str,
    timeout_ms: u64,
) -> Result<CommentaryResult, String> {
    let vocab = seed.expressions.join(", ");
    let system = format!(
        "{SYSTEM_PROMPT}\nCharacter: {}\nVocabulary sample: {vocab}",
        seed.persona
    );
    let user = format!("Sentence: \"{sentence}\"\nEmotion: {emotion}\nProvide the injection:");

    let body = serde_json::json!({
        "model":   model,
        "stream":  false,
        "think":   false,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user   },
        ],
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(format!("{url}/api/chat"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ollama_commentary: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("ollama_commentary: HTTP {}", resp.status()));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let raw = data["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    parse_commentary_response(&raw)
}

/// Parse a one-line `PLACEMENT: text` response from Ollama.
pub fn parse_commentary_response(raw: &str) -> Result<CommentaryResult, String> {
    for line in raw.lines() {
        let line = line.trim();
        for (prefix, placement) in [
            ("PREFIX:", Placement::Prefix),
            ("SUFFIX:", Placement::Suffix),
            ("MID:", Placement::Mid),
        ] {
            if let Some(rest) = line.strip_prefix(prefix) {
                let t = rest.trim().to_string();
                if !t.is_empty() {
                    return Ok(CommentaryResult { text: t, placement });
                }
            }
        }
    }
    // Unrecognised format — use first non-empty line as suffix fallback.
    let text = raw
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("empty commentary response".to_string());
    }
    Ok(CommentaryResult {
        text,
        placement: Placement::Suffix,
    })
}

/// Apply a `CommentaryResult` to `text`.
pub fn apply_commentary(text: &str, result: &CommentaryResult) -> String {
    let (stripped, punct) = strip_trailing_punct(text);
    match result.placement {
        Placement::Prefix => {
            let sep = if result.text.ends_with(',') {
                " "
            } else {
                ", "
            };
            format!("{}{sep}{text}", result.text)
        }
        Placement::Suffix => {
            let sep = if result.text.starts_with(',') {
                ""
            } else {
                ", "
            };
            format!("{stripped}{sep}{}{punct}", result.text)
        }
        Placement::Mid => {
            if let Some(idx) = stripped.rfind(',') {
                format!(
                    "{}, {}{}{punct}",
                    &stripped[..idx],
                    result.text,
                    &stripped[idx..]
                )
            } else {
                let sep = if result.text.starts_with(',') {
                    ""
                } else {
                    ", "
                };
                format!("{stripped}{sep}{}{punct}", result.text)
            }
        }
    }
}

fn strip_trailing_punct(s: &str) -> (String, String) {
    let trimmed = s.trim_end();
    let punct_len = trimmed
        .chars()
        .rev()
        .take_while(|c| matches!(c, '.' | '!' | '?'))
        .count();
    if punct_len == 0 {
        return (trimmed.to_string(), ".".to_string());
    }
    let boundary = trimmed.len()
        - trimmed
            .char_indices()
            .rev()
            .nth(punct_len - 1)
            .map_or(0, |(i, _)| trimmed.len() - i);
    (
        trimmed[..boundary].to_string(),
        trimmed[boundary..].to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_suffix() {
        let r = parse_commentary_response("SUFFIX: блядь").unwrap();
        assert_eq!(r.placement, Placement::Suffix);
        assert_eq!(r.text, "блядь");
    }

    #[test]
    fn parse_prefix() {
        let r = parse_commentary_response("PREFIX: Ёпта,").unwrap();
        assert_eq!(r.placement, Placement::Prefix);
        assert_eq!(r.text, "Ёпта,");
    }

    #[test]
    fn parse_mid() {
        let r = parse_commentary_response("MID: ну").unwrap();
        assert_eq!(r.placement, Placement::Mid);
    }

    #[test]
    fn parse_empty_is_err() {
        assert!(parse_commentary_response("").is_err());
    }

    #[test]
    fn parse_fallback_to_suffix() {
        let r = parse_commentary_response("ёпта").unwrap();
        assert_eq!(r.placement, Placement::Suffix);
    }

    #[test]
    fn apply_suffix_preserves_punct() {
        let r = CommentaryResult {
            text: "блядь".into(),
            placement: Placement::Suffix,
        };
        let out = apply_commentary("Деплой прошёл.", &r);
        assert!(out.ends_with('.'));
        assert!(out.contains("блядь"));
    }

    #[test]
    fn apply_prefix_prepends() {
        let r = CommentaryResult {
            text: "Ёпта,".into(),
            placement: Placement::Prefix,
        };
        let out = apply_commentary("сервер упал.", &r);
        assert!(out.starts_with("Ёпта,"));
    }

    #[test]
    fn apply_mid_inserts_before_last_clause() {
        let r = CommentaryResult {
            text: "ну".into(),
            placement: Placement::Mid,
        };
        let out = apply_commentary("Деплой, братан, прошёл.", &r);
        assert!(out.contains("ну"));
    }
}
