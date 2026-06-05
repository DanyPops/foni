//! Emotion arc — recursive decomposition from input emotion to per-stroke shades.
//!
//! 1. Input emotion (SER: arousal, dominance, valence)
//! 2. Persona reaction (transfer function: how this character responds)
//! 3. Overall arc shape (opening → climax → resolution)
//! 4. Per-stroke shade assignment

use super::expression_palette::Shade;
use super::stroke::{Delimiter, Stroke};

/// Raw emotion reading from SER or defaults.
#[derive(Debug, Clone, Copy)]
pub struct Emotion {
    pub arousal: f32,
    pub dominance: f32,
    pub valence: f32,
}

impl Default for Emotion {
    fn default() -> Self {
        Self {
            arousal: 0.5,
            dominance: 0.5,
            valence: 0.5,
        }
    }
}

/// Where a persona rests on an axis when input is neutral.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    Silent,   // 0.1
    Low,      // 0.3
    Mid,      // 0.5
    High,     // 0.7
    VeryHigh, // 0.9
}

impl Anchor {
    pub fn value(self) -> f32 {
        match self {
            Self::Silent => 0.1,
            Self::Low => 0.3,
            Self::Mid => 0.5,
            Self::High => 0.7,
            Self::VeryHigh => 0.9,
        }
    }
}

/// How a persona responds to input on this axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Weight {
    Inverts,   // -1.0 — opposite of input
    Counters,  // -0.5 — mildly opposite
    Ignores,   //  0.0 — stays at anchor
    Dampens,   //  0.3 — barely follows
    Mirrors,   //  1.0 — matches input
    Amplifies, //  1.5 — exaggerates input
}

impl Weight {
    pub fn value(self) -> f32 {
        match self {
            Self::Inverts => -1.0,
            Self::Counters => -0.5,
            Self::Ignores => 0.0,
            Self::Dampens => 0.3,
            Self::Mirrors => 1.0,
            Self::Amplifies => 1.5,
        }
    }
}

/// Per-axis weight: how a persona reacts along one emotion dimension.
///
/// `output = anchor + (input - 0.5) * weight`, clamped to 0–1.
#[derive(Debug, Clone, Copy)]
pub struct AxisWeight {
    pub anchor: f32,
    pub weight: f32,
}

impl AxisWeight {
    pub fn apply(&self, input: f32) -> f32 {
        (self.anchor + (input - 0.5) * self.weight).clamp(0.0, 1.0)
    }
}

/// Sugar: build an AxisWeight from labels.
pub fn axis(anchor: Anchor, weight: Weight) -> AxisWeight {
    AxisWeight {
        anchor: anchor.value(),
        weight: weight.value(),
    }
}

/// How a persona transforms input emotion into their response emotion.
#[derive(Debug, Clone)]
pub struct PersonaReaction {
    pub name: String,
    pub arousal: AxisWeight,
    pub dominance: AxisWeight,
    pub valence: AxisWeight,
}

impl PersonaReaction {
    pub fn react(&self, input: &Emotion) -> Emotion {
        Emotion {
            arousal: self.arousal.apply(input.arousal),
            dominance: self.dominance.apply(input.dominance),
            valence: self.valence.apply(input.valence),
        }
    }
}

/// Diomedes: amplifies energy, always dominant, tends cold.
pub fn diomedes() -> PersonaReaction {
    PersonaReaction {
        name: "diomedes".into(),
        arousal: axis(Anchor::High, Weight::Amplifies), // never calm, amplifies input
        dominance: axis(Anchor::VeryHigh, Weight::Ignores), // always commanding
        valence: axis(Anchor::Low, Weight::Dampens),    // cold, barely warms
    }
}

/// Sidorovich: flat, businesslike, inverts warmth.
pub fn sidorovich() -> PersonaReaction {
    PersonaReaction {
        name: "sidorovich".into(),
        arousal: axis(Anchor::Low, Weight::Dampens), // flat, barely reacts
        dominance: axis(Anchor::Mid, Weight::Dampens), // casually in control
        valence: axis(Anchor::Low, Weight::Counters), // you're happy → he's drier
    }
}

pub fn persona(name: &str) -> PersonaReaction {
    match name {
        "diomedes" => diomedes(),
        "sidorovich" => sidorovich(),
        _ => PersonaReaction {
            name: name.into(),
            arousal: axis(Anchor::Mid, Weight::Mirrors),
            dominance: axis(Anchor::Mid, Weight::Mirrors),
            valence: axis(Anchor::Mid, Weight::Mirrors),
        },
    }
}

/// Position within an arc — controls how the overall emotion bends per stroke.
#[derive(Debug, Clone, Copy)]
pub enum ArcPhase {
    Opening,
    Build,
    Climax,
    Resolution,
}

/// Map a stroke's position to an arc phase.
fn arc_phase(index: usize, total: usize) -> ArcPhase {
    if total <= 1 {
        return ArcPhase::Climax;
    }
    let pct = index as f32 / (total - 1) as f32;
    if pct < 0.2 {
        ArcPhase::Opening
    } else if pct < 0.6 {
        ArcPhase::Build
    } else if pct < 0.85 {
        ArcPhase::Climax
    } else {
        ArcPhase::Resolution
    }
}

/// Bend the overall reaction emotion per arc phase.
fn phase_modulate(base: &Emotion, phase: ArcPhase) -> Emotion {
    match phase {
        ArcPhase::Opening => Emotion {
            arousal: base.arousal * 0.6,
            dominance: base.dominance * 0.8,
            valence: base.valence * 1.2,
        },
        ArcPhase::Build => Emotion {
            arousal: base.arousal * 0.9,
            dominance: base.dominance,
            valence: base.valence,
        },
        ArcPhase::Climax => Emotion {
            arousal: (base.arousal * 1.3).min(1.0),
            dominance: (base.dominance * 1.2).min(1.0),
            valence: base.valence * 0.7,
        },
        ArcPhase::Resolution => Emotion {
            arousal: base.arousal * 0.7,
            dominance: base.dominance * 0.9,
            valence: (base.valence * 1.4).min(1.0),
        },
    }
}

/// Further adjust for the delimiter that ended this stroke.
fn delimiter_nudge(emotion: &Emotion, delimiter: Delimiter) -> Emotion {
    match delimiter {
        Delimiter::Exclamation => Emotion {
            arousal: (emotion.arousal * 1.3).min(1.0),
            dominance: (emotion.dominance * 1.1).min(1.0),
            valence: emotion.valence,
        },
        Delimiter::Question => Emotion {
            arousal: emotion.arousal * 0.9,
            dominance: emotion.dominance * 0.8,
            valence: (emotion.valence * 1.2).min(1.0),
        },
        Delimiter::Ellipsis => Emotion {
            arousal: emotion.arousal * 0.6,
            dominance: emotion.dominance * 0.9,
            valence: emotion.valence,
        },
        Delimiter::Dash => Emotion {
            arousal: (emotion.arousal * 1.1).min(1.0),
            dominance: emotion.dominance,
            valence: emotion.valence * 0.9,
        },
        _ => *emotion,
    }
}

/// A stroke with its resolved emotion.
#[derive(Debug, Clone)]
pub struct PaintedStroke {
    pub text: String,
    pub delimiter: Delimiter,
    pub emotion: Emotion,
}

/// Analyze input text: split into strokes and assign emotion per position.
/// Uses a flat base emotion (from SER) modulated by arc position + delimiter.
pub fn analyze(base: &Emotion, strokes: &[Stroke]) -> Vec<PaintedStroke> {
    strokes
        .iter()
        .enumerate()
        .map(|(i, stroke)| {
            let phase = arc_phase(i, strokes.len());
            let phased = phase_modulate(base, phase);
            let emotion = delimiter_nudge(&phased, stroke.delimiter);
            PaintedStroke {
                text: stroke.text.clone(),
                delimiter: stroke.delimiter,
                emotion,
            }
        })
        .collect()
}

/// Compose output reply: input emotion → persona reaction → arc → per-stroke emotions.
pub fn paint(input: &Emotion, persona_name: &str, strokes: &[Stroke]) -> Vec<PaintedStroke> {
    let base = persona(persona_name).react(input);
    analyze(&base, strokes)
}

/// Map a painted stroke's emotion to shade parameter values.
/// Excitement ← arousal, Assertiveness ← 1-dominance (inverted), Warmth ← valence
pub fn emotion_to_params(e: &Emotion) -> (f32, f32, f32) {
    let excitement = 0.3 + e.arousal * 1.4;
    let assertiveness = 0.6 - e.dominance * 0.5;
    let warmth = 0.3 + e.valence * 1.0;
    (excitement, assertiveness, warmth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stroke::split_strokes;

    // ── DSL labels ──

    #[test]
    fn anchor_values_ordered() {
        assert!(Anchor::Silent.value() < Anchor::Low.value());
        assert!(Anchor::Low.value() < Anchor::Mid.value());
        assert!(Anchor::Mid.value() < Anchor::High.value());
        assert!(Anchor::High.value() < Anchor::VeryHigh.value());
    }

    #[test]
    fn weight_values_ordered() {
        assert!(Weight::Inverts.value() < Weight::Counters.value());
        assert!(Weight::Counters.value() < Weight::Ignores.value());
        assert!(Weight::Ignores.value() < Weight::Dampens.value());
        assert!(Weight::Dampens.value() < Weight::Mirrors.value());
        assert!(Weight::Mirrors.value() < Weight::Amplifies.value());
    }

    #[test]
    fn axis_sugar_matches_manual() {
        let sugar = axis(Anchor::High, Weight::Amplifies);
        let manual = AxisWeight {
            anchor: 0.7,
            weight: 1.5,
        };
        assert!((sugar.anchor - manual.anchor).abs() < 0.01);
        assert!((sugar.weight - manual.weight).abs() < 0.01);
    }

    // ── AxisWeight ──

    #[test]
    fn weight_positive_amplifies() {
        let w = AxisWeight {
            anchor: 0.5,
            weight: 2.0,
        };
        assert!(w.apply(0.8) > 0.8, "should amplify above midpoint");
        assert!(w.apply(0.2) < 0.2, "should amplify below midpoint");
    }

    #[test]
    fn weight_zero_ignores_input() {
        let w = AxisWeight {
            anchor: 0.7,
            weight: 0.0,
        };
        assert!((w.apply(0.0) - 0.7).abs() < 0.01);
        assert!((w.apply(1.0) - 0.7).abs() < 0.01);
    }

    #[test]
    fn weight_negative_inverts() {
        let w = AxisWeight {
            anchor: 0.5,
            weight: -1.0,
        };
        assert!(w.apply(0.8) < 0.5, "high input → below anchor");
        assert!(w.apply(0.2) > 0.5, "low input → above anchor");
    }

    #[test]
    fn weight_clamps_to_unit() {
        let w = AxisWeight {
            anchor: 0.9,
            weight: 3.0,
        };
        assert!(w.apply(1.0) <= 1.0);
        let w2 = AxisWeight {
            anchor: 0.1,
            weight: 3.0,
        };
        assert!(w2.apply(0.0) >= 0.0);
    }

    // ── Persona reactions ──

    #[test]
    fn diomedes_always_dominant() {
        let calm = Emotion {
            arousal: 0.2,
            dominance: 0.2,
            valence: 0.8,
        };
        let r = diomedes().react(&calm);
        assert!(
            r.dominance > 0.7,
            "Diomedes is always dominant, got {:.2}",
            r.dominance
        );
    }

    #[test]
    fn diomedes_amplifies_arousal() {
        let excited = Emotion {
            arousal: 0.8,
            dominance: 0.5,
            valence: 0.5,
        };
        let r = diomedes().react(&excited);
        assert!(r.arousal > excited.arousal, "should amplify");
    }

    #[test]
    fn sidorovich_dampens_arousal() {
        let excited = Emotion {
            arousal: 0.9,
            dominance: 0.5,
            valence: 0.5,
        };
        let r = sidorovich().react(&excited);
        assert!(r.arousal < excited.arousal, "sidorovich stays flat");
    }

    #[test]
    fn arc_single_stroke_is_climax() {
        assert!(matches!(arc_phase(0, 1), ArcPhase::Climax));
    }

    #[test]
    fn arc_first_is_opening() {
        assert!(matches!(arc_phase(0, 10), ArcPhase::Opening));
    }

    #[test]
    fn arc_last_is_resolution() {
        assert!(matches!(arc_phase(9, 10), ArcPhase::Resolution));
    }

    #[test]
    fn exclamation_boosts_arousal() {
        let e = Emotion {
            arousal: 0.5,
            dominance: 0.5,
            valence: 0.5,
        };
        let nudged = delimiter_nudge(&e, Delimiter::Exclamation);
        assert!(nudged.arousal > e.arousal);
    }

    #[test]
    fn ellipsis_dampens_arousal() {
        let e = Emotion {
            arousal: 0.8,
            dominance: 0.5,
            valence: 0.5,
        };
        let nudged = delimiter_nudge(&e, Delimiter::Ellipsis);
        assert!(nudged.arousal < e.arousal);
    }

    #[test]
    fn paint_produces_one_per_stroke() {
        let input = Emotion::default();
        let strokes = split_strokes("Rise, brother! The Emperor protects.");
        let painted = paint(&input, "diomedes", &strokes);
        assert_eq!(painted.len(), strokes.len());
    }

    #[test]
    fn paint_arc_has_variation() {
        let input = Emotion {
            arousal: 0.5,
            dominance: 0.5,
            valence: 0.5,
        };
        let strokes = split_strokes(
            "Julian, hear me — the Imperium demands it! We shall write... and prevail.",
        );
        let painted = paint(&input, "diomedes", &strokes);
        assert!(painted.len() >= 4);

        let arousals: Vec<f32> = painted.iter().map(|p| p.emotion.arousal).collect();
        let min = arousals.iter().cloned().fold(f32::MAX, f32::min);
        let max = arousals.iter().cloned().fold(f32::MIN, f32::max);
        assert!(
            max - min > 0.05,
            "arc should produce variation, got min={min:.2} max={max:.2}"
        );
    }

    // ── analyze (input decomposition) ──

    #[test]
    fn analyze_produces_one_per_stroke() {
        let base = Emotion {
            arousal: 0.7,
            dominance: 0.4,
            valence: 0.8,
        };
        let strokes = split_strokes("Hey, listen — I got an opportunity!");
        let analyzed = analyze(&base, &strokes);
        assert_eq!(analyzed.len(), strokes.len());
    }

    #[test]
    fn analyze_arc_has_variation() {
        let base = Emotion {
            arousal: 0.6,
            dominance: 0.5,
            valence: 0.7,
        };
        let strokes = split_strokes(
            "So basically, I got this opportunity from a friend, and he said we can publish.",
        );
        let analyzed = analyze(&base, &strokes);
        let arousals: Vec<f32> = analyzed.iter().map(|p| p.emotion.arousal).collect();
        let min = arousals.iter().cloned().fold(f32::MAX, f32::min);
        let max = arousals.iter().cloned().fold(f32::MIN, f32::max);
        assert!(
            max - min > 0.01,
            "input arc should vary, got min={min:.3} max={max:.3}"
        );
    }

    #[test]
    fn analyze_and_paint_same_shape() {
        let input = Emotion {
            arousal: 0.6,
            dominance: 0.4,
            valence: 0.8,
        };
        let text = "Brother, hear me — the battle calls!";
        let strokes = split_strokes(text);

        let input_arc = analyze(&input, &strokes);
        let output_arc = paint(&input, "diomedes", &strokes);

        assert_eq!(input_arc.len(), output_arc.len());
        for (i, (inp, out)) in input_arc.iter().zip(output_arc.iter()).enumerate() {
            assert_eq!(inp.text, out.text, "stroke {i} text mismatch");
            assert_eq!(
                inp.delimiter, out.delimiter,
                "stroke {i} delimiter mismatch"
            );
        }
    }

    #[test]
    fn persona_transforms_emotion_not_structure() {
        let input = Emotion {
            arousal: 0.5,
            dominance: 0.3,
            valence: 0.9,
        };
        let strokes = split_strokes("Hello, friend.");

        let raw = analyze(&input, &strokes);
        let diom = paint(&input, "diomedes", &strokes);

        assert_eq!(raw.len(), diom.len());
        // Diomedes should be more dominant than raw
        assert!(
            diom[0].emotion.dominance > raw[0].emotion.dominance,
            "diomedes should amplify dominance"
        );
    }

    #[test]
    fn emotion_to_params_maps_ranges() {
        let calm = Emotion {
            arousal: 0.0,
            dominance: 0.0,
            valence: 0.0,
        };
        let (e, a, w) = emotion_to_params(&calm);
        assert!((e - 0.3).abs() < 0.01);
        assert!((a - 0.6).abs() < 0.01);
        assert!((w - 0.3).abs() < 0.01);

        let peak = Emotion {
            arousal: 1.0,
            dominance: 1.0,
            valence: 1.0,
        };
        let (e, a, w) = emotion_to_params(&peak);
        assert!((e - 1.7).abs() < 0.01);
        assert!((a - 0.1).abs() < 0.01);
        assert!((w - 1.3).abs() < 0.01);
    }
}
