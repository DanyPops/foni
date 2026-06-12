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

    pub fn from_label(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "flat" => Some(Self::Flat),
            "calm" => Some(Self::Calm),
            "moderate" => Some(Self::Moderate),
            "strong" => Some(Self::Strong),
            "intense" => Some(Self::Intense),
            "explosive" => Some(Self::Explosive),
            "extreme" => Some(Self::Extreme),
            _ => None,
        }
    }
}

/// Cfg_weight axis: how tight/controlled the pacing is.
#[derive(Debug, Clone, Copy)]
pub enum Authority {
    Commanding, // 0.10
    Firm,       // 0.20
    Confident,  // 0.30
    Balanced,   // 0.40
    Tentative,  // 0.50
    Loose,      // 0.60
}

impl Authority {
    pub fn value(self) -> f32 {
        match self {
            Self::Commanding => 0.10,
            Self::Firm => 0.20,
            Self::Confident => 0.30,
            Self::Balanced => 0.40,
            Self::Tentative => 0.50,
            Self::Loose => 0.60,
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "commanding" => Some(Self::Commanding),
            "firm" => Some(Self::Firm),
            "confident" => Some(Self::Confident),
            "balanced" => Some(Self::Balanced),
            "tentative" => Some(Self::Tentative),
            "loose" => Some(Self::Loose),
            _ => None,
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

    pub fn from_label(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "frozen" => Some(Self::Frozen),
            "cold" => Some(Self::Cold),
            "cool" => Some(Self::Cool),
            "neutral" => Some(Self::Neutral),
            "warm" => Some(Self::Warm),
            "hot" => Some(Self::Hot),
            _ => None,
        }
    }
}

/// Resolve three `+`-separated axis labels (case-insensitive, order-independent)
/// into `(exaggeration, cfg_weight, temperature)` values.
///
/// Returns `None` if any label is unknown, or two labels map to the same axis.
pub fn resolve_labels(input: &str) -> Option<(f32, f32, f32)> {
    let tokens: Vec<&str> = input.split('+').map(str::trim).collect();
    if tokens.len() != 3 {
        return None;
    }
    let mut energy: Option<Energy> = None;
    let mut authority: Option<Authority> = None;
    let mut tone: Option<Tone> = None;
    for token in &tokens {
        if let Some(e) = Energy::from_label(token) {
            if energy.is_some() {
                return None;
            }
            energy = Some(e);
        } else if let Some(a) = Authority::from_label(token) {
            if authority.is_some() {
                return None;
            }
            authority = Some(a);
        } else if let Some(t) = Tone::from_label(token) {
            if tone.is_some() {
                return None;
            }
            tone = Some(t);
        } else {
            return None;
        }
    }
    Some((energy?.value(), authority?.value(), tone?.value()))
}

/// Sugar: build a Chatterbox shade from labels.
fn cb(name: &str, energy: Energy, grip: Authority, tone: Tone) -> Shade {
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
        use Authority::*;
        use Energy::*;
        use Tone::*;
        Self {
            shades: vec![
                //                 Energy      Authority         Tone
                // ── Quiet end ──────────────────────────────────────────
                cb("whisper", Flat, Loose, Cool),
                cb("deadpan", Flat, Loose, Cold),
                cb("pondering", Flat, Loose, Hot),
                cb("solemn", Flat, Confident, Cold),
                cb("reverent", Energy::Calm, Authority::Tentative, Tone::Warm),
                // ── Mid energy ───────────────────────────────────────
                cb("measured", Energy::Calm, Authority::Balanced, Tone::Neutral),
                cb("riffing", Energy::Calm, Loose, Hot),
                cb("warm", Energy::Moderate, Authority::Balanced, Tone::Warm),
                cb(
                    "curious",
                    Energy::Moderate,
                    Authority::Tentative,
                    Tone::Neutral,
                ),
                cb("awe", Energy::Moderate, Loose, Hot),
                cb("firm", Moderate, Firm, Cool),
                cb("sarcastic", Moderate, Confident, Cold),
                // ── High energy, tight pacing ───────────────────────
                cb("conviction", Strong, Confident, Warm),
                cb("encouraging", Strong, Confident, Hot),
                cb("menacing", Strong, Commanding, Frozen),
                cb("commanding", Intense, Commanding, Cool),
                cb("rallying", Energy::Intense, Authority::Firm, Tone::Neutral),
                cb("triumphant", Intense, Commanding, Warm),
                cb("battle_cry", Explosive, Commanding, Frozen),
                cb("rage", Extreme, Commanding, Frozen),
                // ── High energy, loose pacing (climactic but breathable) ─
                cb("revelation", Strong, Loose, Hot),
                cb(
                    "anthemic",
                    Energy::Intense,
                    Authority::Tentative,
                    Tone::Warm,
                ),
                cb("transcendent", Intense, Loose, Hot),
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
        assert!(Authority::Commanding.value() < Authority::Firm.value());
        assert!(Authority::Firm.value() < Authority::Confident.value());
        assert!(Authority::Confident.value() < Authority::Balanced.value());
        assert!(Authority::Balanced.value() < Authority::Tentative.value());
        assert!(Authority::Tentative.value() < Authority::Loose.value());
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
        let sugar = cb("test", Energy::Intense, Authority::Commanding, Tone::Cool);
        assert!((sugar.params["exaggeration"] - 1.20).abs() < 0.01);
        assert!((sugar.params["cfg_weight"] - 0.10).abs() < 0.01);
        assert!((sugar.params["temperature"] - 0.65).abs() < 0.01);
    }

    #[test]
    fn commanding_shade_matches_labels() {
        let c = cs();
        let s = c.resolve("commanding").unwrap();
        assert!((s.params["exaggeration"] - Energy::Intense.value()).abs() < 0.01);
        assert!((s.params["cfg_weight"] - Authority::Commanding.value()).abs() < 0.01);
    }

    // ── resolve_labels ─────────────────────────────────────────────

    #[test]
    fn labels_canonical_order() {
        let result = resolve_labels("Intense+Loose+Hot").unwrap();
        assert!((result.0 - Energy::Intense.value()).abs() < 0.01);
        assert!((result.1 - Authority::Loose.value()).abs() < 0.01);
        assert!((result.2 - Tone::Hot.value()).abs() < 0.01);
    }

    #[test]
    fn labels_order_independent() {
        let a = resolve_labels("Intense+Loose+Hot").unwrap();
        let b = resolve_labels("Loose+Hot+Intense").unwrap();
        let c = resolve_labels("Hot+Intense+Loose").unwrap();
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn labels_case_insensitive() {
        let upper = resolve_labels("Intense+Loose+Hot").unwrap();
        let lower = resolve_labels("intense+loose+hot").unwrap();
        let mixed = resolve_labels("INTENSE+Loose+hoT").unwrap();
        assert_eq!(upper, lower);
        assert_eq!(lower, mixed);
    }

    #[test]
    fn labels_alias_matches_arithmetic() {
        let c = ChatterboxColorset::default();
        let alias = c.resolve("transcendent").unwrap();
        let labels = resolve_labels("Intense+Loose+Hot").unwrap();
        assert!((alias.params["exaggeration"] - labels.0).abs() < 0.01);
        assert!((alias.params["cfg_weight"] - labels.1).abs() < 0.01);
        assert!((alias.params["temperature"] - labels.2).abs() < 0.01);
    }

    #[test]
    fn labels_rejects_duplicate_axis() {
        assert!(
            resolve_labels("Intense+Strong+Hot").is_none(),
            "two Energy labels must be rejected"
        );
        assert!(
            resolve_labels("Intense+Loose+Commanding").is_none(),
            "two Authority labels must be rejected"
        );
        assert!(
            resolve_labels("Intense+Loose+Hot+Warm").is_none(),
            "four tokens must be rejected"
        );
    }

    #[test]
    fn labels_rejects_unknown_label() {
        assert!(resolve_labels("Unknown+Loose+Hot").is_none());
        assert!(resolve_labels("Intense+Loose+Purple").is_none());
    }

    #[test]
    fn labels_all_axes_required() {
        assert!(resolve_labels("Intense+Loose").is_none(), "only two tokens");
        assert!(resolve_labels("Intense").is_none(), "only one token");
    }

    #[test]
    fn labels_every_energy_value_parses() {
        for (label, expected) in &[
            ("Flat", 0.30f32),
            ("Calm", 0.50),
            ("Moderate", 0.70),
            ("Strong", 1.00),
            ("Intense", 1.20),
            ("Explosive", 1.50),
            ("Extreme", 1.70),
        ] {
            let r = resolve_labels(&format!("{label}+Loose+Hot")).unwrap();
            assert!((r.0 - expected).abs() < 0.01, "{label}");
        }
    }

    #[test]
    fn labels_every_authority_value_parses() {
        for (label, expected) in &[
            ("Commanding", 0.10f32),
            ("Firm", 0.20),
            ("Confident", 0.30),
            ("Balanced", 0.40),
            ("Tentative", 0.50),
            ("Loose", 0.60),
        ] {
            let r = resolve_labels(&format!("Intense+{label}+Hot")).unwrap();
            assert!((r.1 - expected).abs() < 0.01, "{label}");
        }
    }

    #[test]
    fn labels_every_tone_value_parses() {
        for (label, expected) in &[
            ("Frozen", 0.30f32),
            ("Cold", 0.50),
            ("Cool", 0.65),
            ("Neutral", 0.80),
            ("Warm", 1.00),
            ("Hot", 1.20),
        ] {
            let r = resolve_labels(&format!("Intense+Loose+{label}")).unwrap();
            assert!((r.2 - expected).abs() < 0.01, "{label}");
        }
    }
}
