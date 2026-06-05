//! Expression palette — model-owned emotion shades.
//!
//! Each TTS model defines its own parameter axes and shades.
//! Shades are named points in the model's parameter space.
//! The LLM picks shade names; the model resolves them to API values.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A named point in a model's parameter space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shade {
    pub name: String,
    pub params: HashMap<String, f32>,
}

/// A model's complete expression vocabulary.
pub trait Colorset: Send + Sync {
    fn model_name(&self) -> &str;
    fn axes(&self) -> &[&str];
    fn shades(&self) -> &[Shade];

    fn resolve(&self, name: &str) -> Option<&Shade> {
        let lower = name.to_lowercase();
        self.shades().iter().find(|s| s.name == lower)
    }

    fn shade_names(&self) -> Vec<&str> {
        self.shades().iter().map(|s| s.name.as_str()).collect()
    }

    fn palette_prompt(&self) -> String {
        let mut s = format!(
            "Expression shades for {} (use shade names in [brackets]):\n",
            self.model_name()
        );
        for shade in self.shades() {
            s.push_str(&format!("  [{}]", shade.name));
            for axis in self.axes() {
                if let Some(v) = shade.params.get(*axis) {
                    s.push_str(&format!(" {axis}={v:.1}"));
                }
            }
            s.push('\n');
        }
        s
    }
}

fn shade(name: &str, params: &[(&str, f32)]) -> Shade {
    Shade {
        name: name.to_string(),
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    }
}

/// Chatterbox Multilingual — 3 axes: exaggeration, cfg_weight, temperature.
pub struct ChatterboxColorset {
    shades: Vec<Shade>,
}

impl Default for ChatterboxColorset {
    fn default() -> Self {
        Self {
            shades: vec![
                // Low energy
                shade(
                    "whisper",
                    &[
                        ("exaggeration", 0.30),
                        ("cfg_weight", 0.50),
                        ("temperature", 0.60),
                    ],
                ),
                shade(
                    "measured",
                    &[
                        ("exaggeration", 0.50),
                        ("cfg_weight", 0.40),
                        ("temperature", 0.80),
                    ],
                ),
                shade(
                    "solemn",
                    &[
                        ("exaggeration", 0.40),
                        ("cfg_weight", 0.35),
                        ("temperature", 0.45),
                    ],
                ),
                // Mid energy
                shade(
                    "warm",
                    &[
                        ("exaggeration", 0.60),
                        ("cfg_weight", 0.40),
                        ("temperature", 1.10),
                    ],
                ),
                shade(
                    "firm",
                    &[
                        ("exaggeration", 0.70),
                        ("cfg_weight", 0.25),
                        ("temperature", 0.70),
                    ],
                ),
                shade(
                    "curious",
                    &[
                        ("exaggeration", 0.70),
                        ("cfg_weight", 0.45),
                        ("temperature", 0.90),
                    ],
                ),
                shade(
                    "sarcastic",
                    &[
                        ("exaggeration", 0.65),
                        ("cfg_weight", 0.35),
                        ("temperature", 0.50),
                    ],
                ),
                // High energy
                shade(
                    "commanding",
                    &[
                        ("exaggeration", 1.20),
                        ("cfg_weight", 0.15),
                        ("temperature", 0.60),
                    ],
                ),
                shade(
                    "rallying",
                    &[
                        ("exaggeration", 1.30),
                        ("cfg_weight", 0.20),
                        ("temperature", 0.90),
                    ],
                ),
                shade(
                    "menacing",
                    &[
                        ("exaggeration", 1.10),
                        ("cfg_weight", 0.15),
                        ("temperature", 0.35),
                    ],
                ),
                shade(
                    "encouraging",
                    &[
                        ("exaggeration", 0.90),
                        ("cfg_weight", 0.30),
                        ("temperature", 1.20),
                    ],
                ),
                // Peak energy
                shade(
                    "battle_cry",
                    &[
                        ("exaggeration", 1.50),
                        ("cfg_weight", 0.10),
                        ("temperature", 0.40),
                    ],
                ),
                shade(
                    "rage",
                    &[
                        ("exaggeration", 1.70),
                        ("cfg_weight", 0.10),
                        ("temperature", 0.30),
                    ],
                ),
                shade(
                    "triumphant",
                    &[
                        ("exaggeration", 1.40),
                        ("cfg_weight", 0.15),
                        ("temperature", 1.00),
                    ],
                ),
            ],
        }
    }
}

impl Colorset for ChatterboxColorset {
    fn model_name(&self) -> &str {
        "chatterbox"
    }

    fn axes(&self) -> &[&str] {
        &["exaggeration", "cfg_weight", "temperature"]
    }

    fn shades(&self) -> &[Shade] {
        &self.shades
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs() -> ChatterboxColorset {
        ChatterboxColorset::default()
    }

    #[test]
    fn resolve_by_name() {
        let c = cs();
        let s = c.resolve("commanding").unwrap();
        assert!(s.params["exaggeration"] > 1.0);
    }

    #[test]
    fn resolve_case_insensitive() {
        let c = cs();
        assert!(c.resolve("Battle_Cry").is_some());
        assert!(c.resolve("RAGE").is_some());
    }

    #[test]
    fn resolve_unknown() {
        let c = cs();
        assert!(c.resolve("nonexistent").is_none());
    }

    #[test]
    fn all_shades_have_all_axes() {
        let c = cs();
        for shade in c.shades() {
            for axis in c.axes() {
                assert!(
                    shade.params.contains_key(*axis),
                    "{} missing {axis}",
                    shade.name
                );
            }
        }
    }

    #[test]
    fn no_duplicate_names() {
        let c = cs();
        let names = c.shade_names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
    }

    #[test]
    fn palette_prompt_lists_shades() {
        let c = cs();
        let prompt = c.palette_prompt();
        assert!(prompt.contains("[commanding]"));
        assert!(prompt.contains("[whisper]"));
        assert!(prompt.contains("exaggeration"));
    }

    #[test]
    fn axes_count() {
        let c = cs();
        assert_eq!(c.axes().len(), 3);
    }

    #[test]
    fn energy_range_covered() {
        let c = cs();
        let min = c
            .shades()
            .iter()
            .map(|s| s.params["exaggeration"])
            .fold(f32::MAX, f32::min);
        let max = c
            .shades()
            .iter()
            .map(|s| s.params["exaggeration"])
            .fold(f32::MIN, f32::max);
        assert!(min < 0.4);
        assert!(max > 1.5);
    }
}
