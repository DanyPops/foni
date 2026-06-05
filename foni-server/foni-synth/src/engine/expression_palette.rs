//! Expression palette — an emotion canvas for voice painting.
//!
//! **Colors** are emotion families (authority, warmth, intensity, restraint).
//! **Shades** are gradients within each color (firm → commanding → menacing).
//! A **canvas** is a sequence of shades that paints the emotional arc of a reply.
//!
//! Shades are abstract (what you hear). A **Colorset** maps them to model-specific
//! API parameters — different TTS models, same painting language.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Abstract emotion point — what you hear, not how a model produces it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shade {
    pub name: &'static str,
    pub color: &'static str,
    pub excitement: f32,
    pub assertiveness: f32,
    pub warmth: f32,
}

/// Model-specific parameter tensor — the concrete API values a TTS model needs.
#[derive(Debug, Clone, Default)]
pub struct ModelParams {
    pub values: HashMap<&'static str, f32>,
}

impl ModelParams {
    pub fn get(&self, key: &str) -> Option<f32> {
        self.values.get(key).copied()
    }
}

/// A colorset maps abstract shade values to a specific model's parameter space.
pub trait Colorset: Send + Sync {
    fn name(&self) -> &str;
    fn param_names(&self) -> &[&str];
    fn map_shade(&self, shade: &Shade) -> ModelParams;
}

/// Chatterbox Multilingual colorset.
pub struct ChatterboxColorset;

impl Colorset for ChatterboxColorset {
    fn name(&self) -> &str {
        "chatterbox"
    }

    fn param_names(&self) -> &[&str] {
        &["exaggeration", "cfg_weight", "temperature"]
    }

    fn map_shade(&self, shade: &Shade) -> ModelParams {
        let mut p = ModelParams::default();
        p.values.insert("exaggeration", shade.excitement);
        p.values.insert("cfg_weight", shade.assertiveness);
        p.values.insert("temperature", shade.warmth);
        p
    }
}

pub const PALETTE: &[Shade] = &[
    // ── Restraint: quiet, controlled, internal ──
    Shade {
        name: "whisper",
        color: "restraint",
        excitement: 0.30,
        assertiveness: 0.50,
        warmth: 0.60,
    },
    Shade {
        name: "measured",
        color: "restraint",
        excitement: 0.50,
        assertiveness: 0.40,
        warmth: 0.80,
    },
    Shade {
        name: "solemn",
        color: "restraint",
        excitement: 0.40,
        assertiveness: 0.35,
        warmth: 0.45,
    },
    // ── Warmth: friendly, supportive, human ──
    Shade {
        name: "gentle",
        color: "warmth",
        excitement: 0.50,
        assertiveness: 0.45,
        warmth: 1.10,
    },
    Shade {
        name: "warm",
        color: "warmth",
        excitement: 0.60,
        assertiveness: 0.40,
        warmth: 1.10,
    },
    Shade {
        name: "encouraging",
        color: "warmth",
        excitement: 0.90,
        assertiveness: 0.30,
        warmth: 1.20,
    },
    Shade {
        name: "triumphant",
        color: "warmth",
        excitement: 1.40,
        assertiveness: 0.15,
        warmth: 1.00,
    },
    // ── Authority: commanding, decisive, dominant ──
    Shade {
        name: "firm",
        color: "authority",
        excitement: 0.70,
        assertiveness: 0.25,
        warmth: 0.70,
    },
    Shade {
        name: "commanding",
        color: "authority",
        excitement: 1.20,
        assertiveness: 0.15,
        warmth: 0.60,
    },
    Shade {
        name: "menacing",
        color: "authority",
        excitement: 1.10,
        assertiveness: 0.15,
        warmth: 0.35,
    },
    // ── Intensity: raw energy, peak emotion ──
    Shade {
        name: "rallying",
        color: "intensity",
        excitement: 1.30,
        assertiveness: 0.20,
        warmth: 0.90,
    },
    Shade {
        name: "battle_cry",
        color: "intensity",
        excitement: 1.50,
        assertiveness: 0.10,
        warmth: 0.40,
    },
    Shade {
        name: "rage",
        color: "intensity",
        excitement: 1.70,
        assertiveness: 0.10,
        warmth: 0.30,
    },
    // ── Wit: irony, sharpness, playful edge ──
    Shade {
        name: "curious",
        color: "wit",
        excitement: 0.70,
        assertiveness: 0.45,
        warmth: 0.90,
    },
    Shade {
        name: "sarcastic",
        color: "wit",
        excitement: 0.65,
        assertiveness: 0.35,
        warmth: 0.50,
    },
    Shade {
        name: "dry",
        color: "wit",
        excitement: 0.45,
        assertiveness: 0.40,
        warmth: 0.55,
    },
];

/// Look up a shade by name (case-insensitive).
pub fn resolve(name: &str) -> Option<&'static Shade> {
    let lower = name.to_lowercase();
    PALETTE.iter().find(|s| s.name == lower)
}

/// All shade names in the palette.
pub fn shade_names() -> Vec<&'static str> {
    PALETTE.iter().map(|s| s.name).collect()
}

/// All color (family) names, deduplicated.
pub fn color_names() -> Vec<&'static str> {
    let mut names: Vec<&str> = PALETTE.iter().map(|s| s.color).collect();
    names.sort();
    names.dedup();
    names
}

/// Shades belonging to a specific color family.
pub fn shades_of(color: &str) -> Vec<&'static Shade> {
    PALETTE.iter().filter(|s| s.color == color).collect()
}

/// Format palette as LLM prompt instructions.
pub fn palette_prompt() -> String {
    let mut s = String::from("Expression palette (use shade names in [brackets]):\n");
    for color in color_names() {
        s.push_str(&format!("  {color}:"));
        for shade in shades_of(color) {
            s.push_str(&format!(" {}", shade.name));
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_finds_by_name() {
        let s = resolve("commanding").unwrap();
        assert_eq!(s.color, "authority");
        assert!(s.excitement > 1.0);
    }

    #[test]
    fn resolve_case_insensitive() {
        assert!(resolve("Battle_Cry").is_some());
        assert!(resolve("RAGE").is_some());
    }

    #[test]
    fn resolve_unknown_returns_none() {
        assert!(resolve("nonexistent").is_none());
    }

    #[test]
    fn every_color_has_shades() {
        for color in color_names() {
            let shades = shades_of(color);
            assert!(!shades.is_empty(), "{color} has no shades");
        }
    }

    #[test]
    fn colors_cover_energy_spectrum() {
        let restraint = shades_of("restraint");
        let intensity = shades_of("intensity");
        let min_r = restraint
            .iter()
            .map(|s| s.excitement)
            .fold(f32::MAX, f32::min);
        let max_i = intensity
            .iter()
            .map(|s| s.excitement)
            .fold(f32::MIN, f32::max);
        assert!(min_r < 0.5, "restraint should be calm");
        assert!(max_i > 1.4, "intensity should be high energy");
    }

    #[test]
    fn warmth_family_is_warm() {
        for shade in shades_of("warmth") {
            assert!(
                shade.warmth >= 1.0,
                "{} warmth={}",
                shade.name,
                shade.warmth
            );
        }
    }

    #[test]
    fn no_duplicate_names() {
        let names = shade_names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
    }

    #[test]
    fn palette_prompt_mentions_all_colors() {
        let prompt = palette_prompt();
        for color in color_names() {
            assert!(prompt.contains(color), "prompt missing {color}");
        }
    }

    // ── colorset ──

    #[test]
    fn chatterbox_maps_shade_to_three_params() {
        let cs = ChatterboxColorset;
        let shade = resolve("commanding").unwrap();
        let params = cs.map_shade(shade);
        assert_eq!(params.values.len(), 3);
        assert!((params.get("exaggeration").unwrap() - 1.2).abs() < 0.01);
        assert!((params.get("cfg_weight").unwrap() - 0.15).abs() < 0.01);
        assert!((params.get("temperature").unwrap() - 0.6).abs() < 0.01);
    }

    #[test]
    fn chatterbox_param_names() {
        let cs = ChatterboxColorset;
        let names = cs.param_names();
        assert!(names.contains(&"exaggeration"));
        assert!(names.contains(&"cfg_weight"));
        assert!(names.contains(&"temperature"));
    }

    #[test]
    fn every_shade_maps_to_valid_params() {
        let cs = ChatterboxColorset;
        for shade in PALETTE {
            let params = cs.map_shade(shade);
            for name in cs.param_names() {
                assert!(
                    params.get(name).is_some(),
                    "shade {} missing param {name}",
                    shade.name
                );
            }
        }
    }
}
