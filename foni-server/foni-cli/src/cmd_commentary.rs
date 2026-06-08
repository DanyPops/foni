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
            eprintln!("commentary failed: {e}");
            std::process::exit(1);
        }
    }
}
