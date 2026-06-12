use regex::Regex;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Emotion {
    Angry,
    Frustrated,
    Sarcastic,
    Excited,
    Cute,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WordBias {
    Aggressive,
    Commiseration,
    Mockery,
    Excitement,
    Neutral,
}

#[derive(Debug, Clone)]
pub struct EmotionWeights {
    pub mat_multiplier: f64,
    pub interject_multiplier: f64,
    pub heat_min: u8,
    pub word_bias: WordBias,
}

#[derive(Debug, Clone)]
pub struct EmotionReading {
    pub emotion: Emotion,
    pub confidence: f64,
    pub signals: Vec<String>,
}

pub fn peak_weights(emotion: Emotion) -> EmotionWeights {
    match emotion {
        Emotion::Angry => EmotionWeights {
            mat_multiplier: 2.0,
            interject_multiplier: 0.5,
            heat_min: 3,
            word_bias: WordBias::Aggressive,
        },
        Emotion::Frustrated => EmotionWeights {
            mat_multiplier: 1.5,
            interject_multiplier: 1.0,
            heat_min: 2,
            word_bias: WordBias::Commiseration,
        },
        Emotion::Sarcastic => EmotionWeights {
            mat_multiplier: 0.8,
            interject_multiplier: 2.0,
            heat_min: 1,
            word_bias: WordBias::Mockery,
        },
        Emotion::Excited => EmotionWeights {
            mat_multiplier: 0.3,
            interject_multiplier: 3.0,
            heat_min: 1,
            word_bias: WordBias::Excitement,
        },
        Emotion::Cute => EmotionWeights {
            mat_multiplier: 1.5,
            interject_multiplier: 2.5,
            heat_min: 1,
            word_bias: WordBias::Mockery,
        },
        Emotion::Neutral => neutral_weights(),
    }
}

pub fn neutral_weights() -> EmotionWeights {
    EmotionWeights {
        mat_multiplier: 1.0,
        interject_multiplier: 1.0,
        heat_min: 1,
        word_bias: WordBias::Neutral,
    }
}

pub fn emotion_emoji(emotion: Emotion) -> &'static str {
    match emotion {
        Emotion::Angry => "😤",
        Emotion::Frustrated => "😒",
        Emotion::Sarcastic => "🙄",
        Emotion::Excited => "🤩",
        Emotion::Cute => "🫵",
        Emotion::Neutral => "",
    }
}

pub fn detect_emotion(text: &str) -> EmotionReading {
    let mut signals = Vec::new();
    let mut angry = 0i32;
    let mut frustrated = 0i32;
    let mut sarcastic = 0i32;
    let mut excited = 0i32;
    let mut cute = 0i32;

    let words: Vec<&str> = text.split_whitespace().collect();
    let caps_words: Vec<&&str> = words
        .iter()
        .filter(|w| {
            w.len() >= 3
                && w.chars().all(|c| c.is_uppercase() || !c.is_alphabetic())
                && w.chars().any(|c| c.is_uppercase())
        })
        .collect();

    if caps_words.len() >= 2 || (caps_words.len() == 1 && words.len() <= 3) {
        angry += 2;
        signals.push("caps-lock".into());
    }
    if Regex::new(r"[!]{2}").expect("infallible").is_match(text)
        && !Regex::new(r"[!]{3,}").expect("infallible").is_match(text)
    {
        angry += 1;
        signals.push("double-exclamation".into());
    }
    if Regex::new(r"(?i)\b(wtf|fuck|shit|damn|idiot|stupid|useless|broken|crap|hell)\b")
        .expect("infallible")
        .is_match(text)
    {
        angry += 2;
        signals.push("aggressive-en-words".into());
    }
    if Regex::new(r"(?i)(идиот|тупой|сломал|бесполезн|дурак|чёрт|чёрт возьми)")
        .expect("infallible")
        .is_match(text)
    {
        angry += 2;
        signals.push("aggressive-ru-words".into());
    }
    if words.len() < 6
        && Regex::new(r"[!?]").expect("infallible").is_match(text)
        && !Regex::new(r"[!]{3,}").expect("infallible").is_match(text)
    {
        angry += 1;
        signals.push("short-aggressive".into());
    }

    if Regex::new(r"\.{3,}").expect("infallible").is_match(text) {
        frustrated += 2;
        signals.push("ellipsis".into());
    }
    if Regex::new(r"(?i)\b(ugh|argh|sigh|again|still|always|never|every time|seriously|how many)\b")
        .expect("infallible")
        .is_match(text)
    {
        frustrated += 2;
        signals.push("frustration-en-words".into());
    }
    if Regex::new(r"(?i)(опять|снова|всегда|никогда|серьёзно|сколько раз|ну сколько)")
        .expect("infallible")
        .is_match(text)
    {
        frustrated += 2;
        signals.push("frustration-ru-words".into());
    }
    if Regex::new(r"[?]{2,}").expect("infallible").is_match(text) {
        frustrated += 1;
        signals.push("double-question".into());
    }
    if Regex::new(r"(?i)(why|почему|зачем).{0,30}[?!]")
        .expect("infallible")
        .is_match(text)
    {
        frustrated += 1;
        signals.push("why-question".into());
    }

    if Regex::new(r"(?i)\b(oh great|oh sure|just perfect|wonderful|brilliant|just what i needed|love it|fantastic)\b").expect("infallible").is_match(text) {
        sarcastic += 3; signals.push("sarcasm-en-markers".into());
    }
    if Regex::new(r"(?i)(ну конечно|само собой|ещё бы|надо же|вот уж|неудивительно)")
        .expect("infallible")
        .is_match(text)
    {
        sarcastic += 3;
        signals.push("sarcasm-ru-markers".into());
    }
    if Regex::new(r"(?i)\b(obviously|clearly|of course|totally|definitely|naturally)\b")
        .expect("infallible")
        .is_match(text)
    {
        sarcastic += 1;
        signals.push("irony-adverbs".into());
    }
    if Regex::new(r"(?i)\b(thanks for nothing|great job|well done|nice work)\b")
        .expect("infallible")
        .is_match(text)
    {
        sarcastic += 2;
        signals.push("backhanded-compliment".into());
    }

    if Regex::new(r"[!]{3,}").expect("infallible").is_match(text) {
        excited += 2;
        signals.push("triple-exclamation".into());
    }
    if Regex::new(
        r"(?i)\b(amazing|awesome|incredible|genius|perfect|exactly|love this|this is it)\b",
    )
    .expect("infallible")
    .is_match(text)
    {
        excited += 2;
        signals.push("excited-en-words".into());
    }
    if Regex::new(r"(?i)(отлично|шикарно|гениально|невероятно|обалдеть|класс|круто)")
        .expect("infallible")
        .is_match(text)
    {
        excited += 2;
        signals.push("excited-ru-words".into());
    }
    if Regex::new(r"[🔥💯🎉✨🚀😍🤯⚡]")
        .expect("infallible")
        .is_match(text)
    {
        excited += 3;
        signals.push("excitement-emoji".into());
    }
    if Regex::new(r"(?i)\b(omg|omfg|holy shit|no way|wow)\b")
        .expect("infallible")
        .is_match(text)
    {
        excited += 1;
        signals.push("excited-en-interjections".into());
    }

    if Regex::new(r"(?i)\b(please|kindly|would you mind|could you possibly|pretty please|if you don't mind|sorry to bother)\b").expect("infallible").is_match(text) {
        cute += 3; signals.push("over-polite-en".into());
    }
    if Regex::new(
        r"(?i)(пожалуйста|не могли бы вы|будьте добры|извините что беспокою|спасибо большое)",
    )
    .expect("infallible")
    .is_match(text)
    {
        cute += 3;
        signals.push("over-polite-ru".into());
    }
    if Regex::new(r"(?i)\b(uwu|owo|hehe|teehee|heehee)\b")
        .expect("infallible")
        .is_match(text)
    {
        cute += 4;
        signals.push("uwu-energy".into());
    }
    if Regex::new(r"[💕💖💗💓💞❤️🥺👉👈😊🙏🫶]")
        .expect("infallible")
        .is_match(text)
    {
        cute += 2;
        signals.push("cute-emoji".into());
    }
    if Regex::new(r"(?i)\b(you're amazing|you're the best|you're so smart|so helpful)\b")
        .expect("infallible")
        .is_match(text)
    {
        cute += 2;
        signals.push("excessive-flattery".into());
    }
    if text.contains('~') {
        cute += 1;
        signals.push("tilde-suffix".into());
    }

    let scores = [
        (Emotion::Angry, angry),
        (Emotion::Frustrated, frustrated),
        (Emotion::Sarcastic, sarcastic),
        (Emotion::Excited, excited),
        (Emotion::Cute, cute),
    ];

    let (top_emotion, top_score) = scores
        .iter()
        .max_by_key(|(_, s)| *s)
        .copied()
        .unwrap_or((Emotion::Neutral, 0));

    if top_score == 0 {
        return EmotionReading {
            emotion: Emotion::Neutral,
            confidence: 0.0,
            signals: vec![],
        };
    }

    EmotionReading {
        emotion: top_emotion,
        confidence: (top_score as f64 / 4.0).min(1.0),
        signals,
    }
}

const DEFAULT_HALF_LIFE_MS: f64 = 5.0 * 60.0 * 1000.0;
const FAST_DECAY_HALF_LIFE_MS: f64 = 60.0 * 1000.0;
const INTENSITY_FLOOR: f64 = 0.1;

#[derive(Debug, Clone)]
pub struct EmotionState {
    pub emotion: Emotion,
    pub intensity: f64,
    pub detected_at_ms: f64,
    pub half_life_ms: f64,
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}

pub fn neutral_state() -> EmotionState {
    EmotionState {
        emotion: Emotion::Neutral,
        intensity: 0.0,
        detected_at_ms: now_ms(),
        half_life_ms: DEFAULT_HALF_LIFE_MS,
    }
}

pub fn current_intensity(state: &EmotionState, now: f64) -> f64 {
    if state.emotion == Emotion::Neutral {
        return 0.0;
    }
    let elapsed = now - state.detected_at_ms;
    let decayed = state.intensity * 2.0_f64.powf(-elapsed / state.half_life_ms);
    if decayed < INTENSITY_FLOOR {
        0.0
    } else {
        decayed
    }
}

pub fn effective_weights(state: &EmotionState, now: f64) -> EmotionWeights {
    let intensity = current_intensity(state, now);
    if intensity == 0.0 {
        return neutral_weights();
    }

    let peak = peak_weights(state.emotion);
    let nw = neutral_weights();

    EmotionWeights {
        mat_multiplier: lerp(nw.mat_multiplier, peak.mat_multiplier, intensity),
        interject_multiplier: lerp(
            nw.interject_multiplier,
            peak.interject_multiplier,
            intensity,
        ),
        heat_min: if intensity >= 0.5 {
            peak.heat_min
        } else {
            nw.heat_min
        },
        word_bias: if intensity >= 0.5 {
            peak.word_bias
        } else {
            nw.word_bias
        },
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

pub fn update_emotion_state(
    current: &EmotionState,
    reading: &EmotionReading,
    now: f64,
) -> EmotionState {
    if reading.emotion == Emotion::Neutral || reading.confidence == 0.0 {
        return EmotionState {
            half_life_ms: current.half_life_ms.min(FAST_DECAY_HALF_LIFE_MS),
            ..current.clone()
        };
    }

    let cur_intensity = current_intensity(current, now);

    if reading.emotion == current.emotion {
        return EmotionState {
            emotion: current.emotion,
            intensity: (cur_intensity + reading.confidence * 0.35).min(1.0),
            detected_at_ms: now,
            half_life_ms: DEFAULT_HALF_LIFE_MS,
        };
    }

    if reading.confidence >= cur_intensity {
        return EmotionState {
            emotion: reading.emotion,
            intensity: reading.confidence,
            detected_at_ms: now,
            half_life_ms: DEFAULT_HALF_LIFE_MS,
        };
    }

    EmotionState {
        intensity: cur_intensity * 0.6,
        half_life_ms: FAST_DECAY_HALF_LIFE_MS,
        ..current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_caps_lock() {
        let r = detect_emotion("WHAT THE HELL");
        assert_eq!(r.emotion, Emotion::Angry);
        assert!(r.signals.contains(&"caps-lock".to_string()));
    }

    #[test]
    fn detects_double_exclamation() {
        let r = detect_emotion("This is broken!!");
        assert!(r.signals.contains(&"double-exclamation".to_string()));
    }

    #[test]
    fn detects_aggressive_en_words() {
        let r = detect_emotion("this is stupid and broken");
        assert_eq!(r.emotion, Emotion::Angry);
    }

    #[test]
    fn detects_aggressive_ru_words() {
        let r = detect_emotion("ты тупой идиот");
        assert_eq!(r.emotion, Emotion::Angry);
    }

    #[test]
    fn detects_ellipsis() {
        let r = detect_emotion("well... I guess...");
        assert_eq!(r.emotion, Emotion::Frustrated);
        assert!(r.signals.contains(&"ellipsis".to_string()));
    }

    #[test]
    fn detects_frustration_en() {
        let r = detect_emotion("ugh, not again, seriously");
        assert_eq!(r.emotion, Emotion::Frustrated);
    }

    #[test]
    fn detects_frustration_ru() {
        let r = detect_emotion("опять эта ошибка, ну сколько можно");
        assert_eq!(r.emotion, Emotion::Frustrated);
    }

    #[test]
    fn detects_double_question() {
        let r = detect_emotion("why is this happening??");
        assert!(r.signals.contains(&"double-question".to_string()));
    }

    #[test]
    fn detects_sarcasm_en() {
        let r = detect_emotion("oh great, just what I needed");
        assert_eq!(r.emotion, Emotion::Sarcastic);
    }

    #[test]
    fn detects_sarcasm_ru() {
        let r = detect_emotion("ну конечно, надо же");
        assert_eq!(r.emotion, Emotion::Sarcastic);
    }

    #[test]
    fn detects_irony_adverbs() {
        let r = detect_emotion("obviously this is the best solution");
        assert!(r.signals.contains(&"irony-adverbs".to_string()));
    }

    #[test]
    fn detects_backhanded_compliment() {
        let r = detect_emotion("well done, thanks for nothing");
        assert_eq!(r.emotion, Emotion::Sarcastic);
    }

    #[test]
    fn detects_triple_exclamation() {
        let r = detect_emotion("this is amazing!!!");
        assert_eq!(r.emotion, Emotion::Excited);
    }

    #[test]
    fn detects_excited_en() {
        let r = detect_emotion("this is amazing and awesome");
        assert_eq!(r.emotion, Emotion::Excited);
    }

    #[test]
    fn detects_excited_ru() {
        let r = detect_emotion("отлично, невероятно круто");
        assert_eq!(r.emotion, Emotion::Excited);
    }

    #[test]
    fn detects_excitement_emoji() {
        let r = detect_emotion("let's go 🔥🚀");
        assert_eq!(r.emotion, Emotion::Excited);
    }

    #[test]
    fn detects_over_polite_en() {
        let r = detect_emotion("please, if you don't mind, could you possibly help");
        assert_eq!(r.emotion, Emotion::Cute);
    }

    #[test]
    fn detects_over_polite_ru() {
        let r = detect_emotion("пожалуйста, будьте добры помочь");
        assert_eq!(r.emotion, Emotion::Cute);
    }

    #[test]
    fn detects_uwu() {
        let r = detect_emotion("hewwo uwu");
        assert_eq!(r.emotion, Emotion::Cute);
    }

    #[test]
    fn detects_cute_emoji() {
        let r = detect_emotion("thank you 🥺💕");
        assert_eq!(r.emotion, Emotion::Cute);
    }

    #[test]
    fn detects_tilde() {
        let r = detect_emotion("hello~");
        assert!(r.signals.contains(&"tilde-suffix".to_string()));
    }

    #[test]
    fn neutral_for_plain_text() {
        let r = detect_emotion("Can you refactor the config module?");
        assert_eq!(r.emotion, Emotion::Neutral);
        assert_eq!(r.confidence, 0.0);
    }

    #[test]
    fn neutral_state_has_zero_intensity() {
        let s = neutral_state();
        assert_eq!(s.emotion, Emotion::Neutral);
        assert_eq!(current_intensity(&s, now_ms()), 0.0);
    }

    #[test]
    fn intensity_decays_over_time() {
        let s = EmotionState {
            emotion: Emotion::Angry,
            intensity: 1.0,
            detected_at_ms: 0.0,
            half_life_ms: DEFAULT_HALF_LIFE_MS,
        };
        let at_half = current_intensity(&s, DEFAULT_HALF_LIFE_MS);
        assert!((at_half - 0.5).abs() < 0.01);
    }

    #[test]
    fn intensity_below_floor_becomes_zero() {
        let s = EmotionState {
            emotion: Emotion::Angry,
            intensity: 0.2,
            detected_at_ms: 0.0,
            half_life_ms: 1000.0,
        };
        let far_future = current_intensity(&s, 100_000.0);
        assert_eq!(far_future, 0.0);
    }

    #[test]
    fn effective_weights_at_full_intensity() {
        let s = EmotionState {
            emotion: Emotion::Angry,
            intensity: 1.0,
            detected_at_ms: 100.0,
            half_life_ms: DEFAULT_HALF_LIFE_MS,
        };
        let w = effective_weights(&s, 100.0);
        assert!((w.mat_multiplier - 2.0).abs() < 0.01);
        assert_eq!(w.word_bias, WordBias::Aggressive);
    }

    #[test]
    fn effective_weights_at_zero_returns_neutral() {
        let s = neutral_state();
        let w = effective_weights(&s, now_ms());
        assert!((w.mat_multiplier - 1.0).abs() < 0.01);
        assert_eq!(w.word_bias, WordBias::Neutral);
    }

    #[test]
    fn update_reinforces_same_emotion() {
        let s = EmotionState {
            emotion: Emotion::Angry,
            intensity: 0.5,
            detected_at_ms: 100.0,
            half_life_ms: DEFAULT_HALF_LIFE_MS,
        };
        let reading = EmotionReading {
            emotion: Emotion::Angry,
            confidence: 0.8,
            signals: vec![],
        };
        let next = update_emotion_state(&s, &reading, 100.0);
        assert_eq!(next.emotion, Emotion::Angry);
        assert!(next.intensity > s.intensity);
    }

    #[test]
    fn update_shifts_on_stronger_emotion() {
        let s = EmotionState {
            emotion: Emotion::Frustrated,
            intensity: 0.3,
            detected_at_ms: 0.0,
            half_life_ms: DEFAULT_HALF_LIFE_MS,
        };
        let reading = EmotionReading {
            emotion: Emotion::Angry,
            confidence: 0.8,
            signals: vec![],
        };
        let next = update_emotion_state(&s, &reading, 100.0);
        assert_eq!(next.emotion, Emotion::Angry);
        assert!((next.intensity - 0.8).abs() < 0.01);
    }

    #[test]
    fn update_neutral_reading_accelerates_decay() {
        let s = EmotionState {
            emotion: Emotion::Angry,
            intensity: 0.8,
            detected_at_ms: 0.0,
            half_life_ms: DEFAULT_HALF_LIFE_MS,
        };
        let reading = EmotionReading {
            emotion: Emotion::Neutral,
            confidence: 0.0,
            signals: vec![],
        };
        let next = update_emotion_state(&s, &reading, 100.0);
        assert!(next.half_life_ms < DEFAULT_HALF_LIFE_MS);
    }

    #[test]
    fn update_weaker_emotion_blends() {
        let s = EmotionState {
            emotion: Emotion::Angry,
            intensity: 0.9,
            detected_at_ms: 100.0,
            half_life_ms: DEFAULT_HALF_LIFE_MS,
        };
        let reading = EmotionReading {
            emotion: Emotion::Excited,
            confidence: 0.3,
            signals: vec![],
        };
        let next = update_emotion_state(&s, &reading, 100.0);
        assert_eq!(next.emotion, Emotion::Angry);
        assert!(next.intensity < s.intensity);
    }

    #[test]
    fn emoji_for_each_emotion() {
        assert_eq!(emotion_emoji(Emotion::Angry), "😤");
        assert_eq!(emotion_emoji(Emotion::Neutral), "");
        assert!(!emotion_emoji(Emotion::Excited).is_empty());
    }

    #[test]
    fn peak_weights_differ_per_emotion() {
        let angry = peak_weights(Emotion::Angry);
        let excited = peak_weights(Emotion::Excited);
        assert!(angry.mat_multiplier > excited.mat_multiplier);
        assert!(excited.interject_multiplier > angry.interject_multiplier);
    }
}
