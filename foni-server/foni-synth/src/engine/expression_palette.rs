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

// ── Chatterbox axis labels ──
// Each axis gets its own label enum — what you hear, not the API name.

/// Exaggeration axis: how animated/energetic the voice sounds.
#[derive(Debug, Clone, Copy)]
pub enum Energy {
    Flat,      // 0.30
    Calm,      // 0.50
    Moderate,  // 0.70
    Strong,    // 1.00
    Intense,   // 1.20
    Explosive, // 1.50
    Extreme,   // 1.70
}

impl Energy {
    pub fn value(self) -> f32 {
        match self {
            Self::Flat => 0.30,
            Self::Calm => 0.50,
            Self::Moderate => 0.70,
            Self::Strong => 1.00,
            Self::Intense => 1.20,
            Self::Explosive => 1.50,
            Self::Extreme => 1.70,
        }
    }
}

/// Cfg_weight axis: how tight/controlled the pacing is.
#[derive(Debug, Clone, Copy)]
pub enum Grip {
    Commanding, // 0.10
    Firm,       // 0.20
    Confident,  // 0.30
    Neutral,    // 0.40
    Tentative,  // 0.50
    Loose,      // 0.60
}

impl Grip {
    pub fn value(self) -> f32 {
        match self {
            Self::Commanding => 0.10,
            Self::Firm => 0.20,
            Self::Confident => 0.30,
            Self::Neutral => 0.40,
            Self::Tentative => 0.50,
            Self::Loose => 0.60,
        }
    }
}

/// Temperature axis: emotional tone from hostile to friendly.
#[derive(Debug, Clone, Copy)]
pub enum Tone {
    Frozen,  // 0.30
    Cold,    // 0.50
    Cool,    // 0.65
    Neutral, // 0.80
    Warm,    // 1.00
    Hot,     // 1.20
}

impl Tone {
    pub fn value(self) -> f32 {
        match self {
            Self::Frozen => 0.30,
            Self::Cold => 0.50,
            Self::Cool => 0.65,
            Self::Neutral => 0.80,
            Self::Warm => 1.00,
            Self::Hot => 1.20,
        }
    }
}

/// Sugar: build a Chatterbox shade from labels.
fn cb(name: &str, energy: Energy, grip: Grip, tone: Tone) -> Shade {
    Shade {
        name: name.to_string(),
        params: HashMap::from([
            ("exaggeration".to_string(), energy.value()),
            ("cfg_weight".to_string(), grip.value()),
            ("temperature".to_string(), tone.value()),
        ]),
    }
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

/// Chatterbox Multilingual — 3 axes: exaggeration, cfg_weight, temperature.
pub struct ChatterboxColorset {
    shades: Vec<Shade>,
}

impl Default for ChatterboxColorset {
    fn default() -> Self {
        use Energy::*;
        use Grip::*;
        use Tone::*;
        Self {
            shades: vec![
                //                 Energy      Grip         Tone
                cb("whisper", Flat, Loose, Cool),
                cb("measured", Energy::Calm, Grip::Neutral, Tone::Neutral),
                cb("solemn", Flat, Confident, Cold),
                cb("warm", Energy::Moderate, Grip::Neutral, Tone::Warm),
                cb("firm", Moderate, Firm, Cool),
                cb("curious", Energy::Moderate, Grip::Tentative, Tone::Neutral),
                cb("sarcastic", Moderate, Confident, Cold),
                cb("commanding", Intense, Commanding, Cool),
                cb("rallying", Energy::Intense, Grip::Firm, Tone::Neutral),
                cb("menacing", Strong, Commanding, Frozen),
                cb("encouraging", Strong, Confident, Hot),
                cb("battle_cry", Explosive, Commanding, Frozen),
                cb("rage", Extreme, Commanding, Frozen),
                cb("triumphant", Intense, Commanding, Warm),
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

    // ── DSL labels ──

    #[test]
    fn energy_labels_ordered() {
        assert!(Energy::Flat.value() < Energy::Calm.value());
        assert!(Energy::Calm.value() < Energy::Moderate.value());
        assert!(Energy::Moderate.value() < Energy::Strong.value());
        assert!(Energy::Strong.value() < Energy::Intense.value());
        assert!(Energy::Intense.value() < Energy::Explosive.value());
        assert!(Energy::Explosive.value() < Energy::Extreme.value());
    }

    #[test]
    fn grip_labels_ordered() {
        assert!(Grip::Commanding.value() < Grip::Firm.value());
        assert!(Grip::Firm.value() < Grip::Confident.value());
        assert!(Grip::Confident.value() < Grip::Neutral.value());
        assert!(Grip::Neutral.value() < Grip::Tentative.value());
        assert!(Grip::Tentative.value() < Grip::Loose.value());
    }

    #[test]
    fn tone_labels_ordered() {
        assert!(Tone::Frozen.value() < Tone::Cold.value());
        assert!(Tone::Cold.value() < Tone::Cool.value());
        assert!(Tone::Cool.value() < Tone::Neutral.value());
        assert!(Tone::Neutral.value() < Tone::Warm.value());
        assert!(Tone::Warm.value() < Tone::Hot.value());
    }

    #[test]
    fn cb_sugar_matches_manual() {
        let sugar = cb("test", Energy::Intense, Grip::Commanding, Tone::Cool);
        assert!((sugar.params["exaggeration"] - 1.20).abs() < 0.01);
        assert!((sugar.params["cfg_weight"] - 0.10).abs() < 0.01);
        assert!((sugar.params["temperature"] - 0.65).abs() < 0.01);
    }

    #[test]
    fn commanding_shade_matches_labels() {
        let c = cs();
        let s = c.resolve("commanding").unwrap();
        assert!((s.params["exaggeration"] - Energy::Intense.value()).abs() < 0.01);
        assert!((s.params["cfg_weight"] - Grip::Commanding.value()).abs() < 0.01);
    }
}
