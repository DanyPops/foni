use crate::engine::stress::StressMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoniConfig {
    pub enabled: bool,
    pub voice: String,
    pub speed: f64,
    pub input_lang: Lang,
    pub output_lang: Lang,

    pub mat_enabled: bool,
    pub mat_prob: f64,
    pub mat_stretch: f64,
    pub mat_cooldown_ms: u64,

    pub interject_enabled: bool,
    pub interject_prob: f64,
    pub interject_cooldown_ms: u64,

    pub rvc_enabled: bool,
    pub rvc_url: String,
    pub rvc_model: String,

    pub prosody_enabled: bool,

    pub ollama_url: String,
    pub ollama_model: String,

    /// Use Ollama to generate contextual commentary instead of static word pools.
    /// Falls back to static injection on timeout or error.
    #[serde(default)]
    pub llm_commentary_enabled: bool,

    /// Maximum milliseconds to wait for a commentary response before falling back.
    #[serde(default = "default_llm_commentary_timeout_ms")]
    pub llm_commentary_timeout_ms: u64,

    /// Skip external calls (Ollama, synthesis, playback). For testing.
    pub dry_run: bool,

    /// Stress annotation backend.
    #[serde(default)]
    pub stress_mode: StressMode,

    /// URL of the ruaccent sidecar (used when stress_mode = Ruaccent).
    #[serde(default = "default_ruaccent_url")]
    pub ruaccent_url: String,

    /// Translation backend: Ollama (local) or Nllb (Modal).
    #[serde(default)]
    pub translate_backend: TranslateBackend,

    /// URL of the NLLB Modal endpoint.
    #[serde(default = "default_nllb_url")]
    pub nllb_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranslateBackend {
    #[default]
    Ollama,
    Nllb,
}

impl std::str::FromStr for TranslateBackend {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "nllb" | "modal" => Self::Nllb,
            _ => Self::Ollama,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    En,
    Ru,
}

impl Default for FoniConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            voice: "ru".into(),
            speed: 1.0,
            input_lang: Lang::En,
            output_lang: Lang::Ru,

            mat_enabled: true,
            mat_prob: 0.35,
            mat_stretch: 0.5,
            mat_cooldown_ms: 20_000,

            interject_enabled: true,
            interject_prob: 0.25,
            interject_cooldown_ms: 12_000,

            rvc_enabled: true,
            rvc_url: "http://localhost:5050".into(),
            rvc_model: "sidorovich".into(),

            prosody_enabled: true,

            ollama_url: "http://localhost:11434".into(),
            ollama_model: "qwen3:1.7b".into(),
            llm_commentary_enabled: false,
            llm_commentary_timeout_ms: default_llm_commentary_timeout_ms(),
            dry_run: false,
            stress_mode: StressMode::Dict,
            ruaccent_url: default_ruaccent_url(),
            translate_backend: TranslateBackend::Ollama,
            nllb_url: default_nllb_url(),
        }
    }
}

fn default_llm_commentary_timeout_ms() -> u64 {
    800
}

fn default_ruaccent_url() -> String {
    "http://localhost:8765/annotate".into()
}

fn default_nllb_url() -> String {
    "https://dpopsuev--foni-translate-nllbtranslator-translate.modal.run".into()
}

pub const PREWARM_RU: &[&str] = &[
    "Да.",
    "Нет.",
    "Хорошо.",
    "Понял.",
    "Окей.",
    "Готово.",
    "Сейчас.",
    "Подожди.",
    "Конечно.",
    "Блядь.",
    "Пиздец.",
    "Ёпта.",
    "Сука.",
    "Ого!",
    "Ах!",
    "Ух!",
    "Эх.",
];

pub const FILLER_PHRASES: &[&str] = &[
    "Мм...",
    "Хм...",
    "Эм...",
    "Ну...",
    "Так...",
    "Значит...",
    "Это...",
    "Так, так, так...",
    "Ну, значит...",
    "Сейчас, сейчас...",
    "Дай подумать...",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let cfg = FoniConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.input_lang, Lang::En);
        assert_eq!(cfg.output_lang, Lang::Ru);
        assert!(cfg.mat_prob > 0.0 && cfg.mat_prob <= 1.0);
        assert!(cfg.interject_prob > 0.0 && cfg.interject_prob <= 1.0);
    }

    #[test]
    fn config_serializes_to_json() {
        let cfg = FoniConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("sidorovich"));
        assert!(json.contains("\"en\""));
    }

    #[test]
    fn config_roundtrips() {
        let cfg = FoniConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: FoniConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.rvc_model, cfg.rvc_model);
        assert_eq!(parsed.mat_prob, cfg.mat_prob);
    }

    #[test]
    fn prewarm_phrases_nonempty() {
        assert!(!PREWARM_RU.is_empty());
        assert!(PREWARM_RU.len() >= 10);
    }

    #[test]
    fn filler_phrases_nonempty() {
        assert!(!FILLER_PHRASES.is_empty());
        assert!(FILLER_PHRASES.len() >= 5);
    }
}
