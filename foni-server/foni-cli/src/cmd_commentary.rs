use foni_client::commentary::{apply_commentary, ollama_commentary, CharacterSeed};
use std::path::PathBuf;
use tracing::warn;

const DEFAULT_PERSONA: &str = "\
Ты — Сидорович, советский торговец из Зоны. Грубый, циничный, бывалый. \
Говоришь коротко. Реагируешь на то, что происходит — хорошее, плохое, \
удивительное. Матерный язык — норма, не исключение. \
Один вброс — одно слово или фраза.";

const DEFAULT_EXPRESSIONS: &[&str] = &[
    "блядь",
    "ёпта",
    "нихуя себе",
    "охуеть",
    "заебись",
    "чё за дела",
    "тьфу",
    "ух ты",
    "мудак",
    "твою мать",
    "конкретно",
    "по-любому",
];

#[derive(Debug, serde::Deserialize)]
struct LexiconFile {
    character_seed: LexiconSeed,
}

#[derive(Debug, serde::Deserialize)]
struct LexiconSeed {
    persona: String,
    expressions: Vec<String>,
}

fn load_seed(path: Option<&PathBuf>) -> (String, Vec<String>) {
    let yaml = path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .or_else(|| {
            std::env::var("FONI_LEXICON_PATH")
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
        });

    if let Some(yaml) = yaml {
        match serde_yaml::from_str::<LexiconFile>(&yaml) {
            Ok(f) => return (f.character_seed.persona, f.character_seed.expressions),
            Err(e) => warn!(error = %e, "failed to parse lexicon.yaml, using defaults"),
        }
    }

    (
        DEFAULT_PERSONA.to_string(),
        DEFAULT_EXPRESSIONS.iter().map(|s| s.to_string()).collect(),
    )
}

pub async fn cmd_commentary(
    text: &str,
    emotion: &str,
    ollama_url: &str,
    model: &str,
    timeout_ms: u64,
    lexicon: Option<&PathBuf>,
) {
    let (persona, exprs) = load_seed(lexicon);
    let expr_refs: Vec<&str> = exprs.iter().map(String::as_str).collect();
    let seed = CharacterSeed {
        persona: &persona,
        expressions: &expr_refs,
    };

    match ollama_commentary(text, emotion, &seed, ollama_url, model, timeout_ms).await {
        Ok(result) => {
            let modified = apply_commentary(text, &result);
            println!("original:  {text}");
            println!("placement: {}", result.placement);
            println!("injection: {}", result.text);
            println!("result:    {modified}");
        }
        Err(e) => {
            tracing::error!(error = %e, "commentary failed");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn valid_yaml(persona: &str, expressions: &[&str]) -> String {
        let exprs = expressions
            .iter()
            .map(|e| format!("    - {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("character_seed:\n  persona: {persona}\n  expressions:\n{exprs}\n")
    }

    #[test]
    fn defaults_when_no_path_and_no_env() {
        // Ensure FONI_LEXICON_PATH is not set for this test.
        std::env::remove_var("FONI_LEXICON_PATH");
        let (persona, exprs) = load_seed(None);
        assert!(!persona.is_empty(), "default persona should be set");
        assert!(
            exprs.len() >= 6,
            "default expressions should have at least 6 entries, got {}",
            exprs.len()
        );
    }

    #[test]
    fn reads_explicit_path() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        let yaml = valid_yaml("Тест-персонаж", &["тест", "проверка"]);
        write!(f, "{yaml}").unwrap();
        let path = PathBuf::from(f.path());
        let (persona, exprs) = load_seed(Some(&path));
        assert_eq!(persona, "Тест-персонаж");
        assert_eq!(exprs, vec!["тест", "проверка"]);
    }

    #[test]
    fn reads_foni_lexicon_path_env() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        let yaml = valid_yaml("Среда-персонаж", &["раз", "два"]);
        write!(f, "{yaml}").unwrap();
        std::env::set_var("FONI_LEXICON_PATH", f.path());
        let (persona, exprs) = load_seed(None);
        std::env::remove_var("FONI_LEXICON_PATH");
        assert_eq!(persona, "Среда-персонаж");
        assert_eq!(exprs, vec!["раз", "два"]);
    }

    #[test]
    fn falls_back_on_malformed_yaml() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "not: valid: yaml: [").unwrap();
        let path = PathBuf::from(f.path());
        // Should not panic — falls back silently to defaults.
        let (persona, exprs) = load_seed(Some(&path));
        assert!(!persona.is_empty());
        assert!(!exprs.is_empty());
    }

    #[test]
    fn explicit_path_takes_priority_over_env() {
        let mut env_file = tempfile::NamedTempFile::new().unwrap();
        let mut explicit_file = tempfile::NamedTempFile::new().unwrap();
        write!(env_file, "{}", valid_yaml("Из-енв", &["енв"])).unwrap();
        write!(explicit_file, "{}", valid_yaml("Из-файла", &["файл"])).unwrap();
        std::env::set_var("FONI_LEXICON_PATH", env_file.path());
        let path = PathBuf::from(explicit_file.path());
        let (persona, _) = load_seed(Some(&path));
        std::env::remove_var("FONI_LEXICON_PATH");
        assert_eq!(persona, "Из-файла", "explicit path should win over env");
    }
}
