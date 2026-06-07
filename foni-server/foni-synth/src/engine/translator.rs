use rand::Rng;
use regex::Regex;
use std::collections::HashMap;

use super::emotion::WordBias;
use super::lexicon::{self, BiasWordSet};

#[derive(Debug, Clone)]
pub struct PlacementScores {
    pub prefix: f64,
    pub mid: f64,
    pub suffix: f64,
}

pub fn score_placement(sentence: &str) -> PlacementScores {
    if sentence.is_empty() {
        return PlacementScores {
            prefix: 0.0,
            mid: 0.0,
            suffix: 0.0,
        };
    }
    let clause_count = sentence.matches(',').count() + 1;
    let word_count = sentence.split_whitespace().count();
    let is_question = sentence.trim().ends_with('?');
    let has_tech = Regex::new(r"(?i)(баг|деплой|пайплайн|коммит|API|HTTP|сервер|лог|тест|билд|фича|фикс|кэш|бэкенд|фронтенд)")
        .expect("infallible")
        .is_match(sentence);

    let suffix: f64 = if is_question {
        0.0
    } else {
        f64::min(0.55 + if word_count > 8 { 0.2 } else { 0.0 }, 1.0)
    };
    let mid: f64 = f64::min((clause_count as f64 - 1.0) * 0.4, 1.0);
    let tech_boost: f64 = if has_tech { 0.2 } else { 0.0 };
    let len_boost: f64 = if word_count > 12 { 0.1 } else { 0.0 };
    let prefix: f64 = f64::min(0.25 + tech_boost + len_boost, 1.0);

    PlacementScores {
        prefix,
        mid,
        suffix,
    }
}

pub struct WordDiversifier {
    counts: HashMap<String, u32>,
}

impl Default for WordDiversifier {
    fn default() -> Self {
        Self::new()
    }
}

impl WordDiversifier {
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    pub fn pick(&mut self, arr: &[&str]) -> String {
        assert!(!arr.is_empty(), "WordDiversifier::pick: empty array");
        if arr.len() == 1 {
            self.increment(arr[0]);
            return arr[0].to_string();
        }

        let weights: Vec<f64> = arr
            .iter()
            .map(|w| 1.0 / (1.0 + *self.counts.get(*w).unwrap_or(&0) as f64))
            .collect();
        let total: f64 = weights.iter().sum();

        let mut rng = rand::thread_rng();
        let mut r = rng.gen::<f64>() * total;
        for (i, w) in weights.iter().enumerate() {
            r -= w;
            if r <= 0.0 {
                self.increment(arr[i]);
                return arr[i].to_string();
            }
        }
        let last = arr[arr.len() - 1];
        self.increment(last);
        last.to_string()
    }

    pub fn reset(&mut self) {
        self.counts.clear();
    }

    pub fn get_count(&self, word: &str) -> u32 {
        *self.counts.get(word).unwrap_or(&0)
    }

    fn increment(&mut self, word: &str) {
        *self.counts.entry(word.to_string()).or_insert(0) += 1;
    }
}

const STRETCH_REPEATS_MIN: usize = 2;
const STRETCH_REPEATS_MAX: usize = 4;

pub fn stretch_expression(expr: &str, repeats: usize) -> String {
    let chars: Vec<char> = expr.chars().collect();
    let mut positions: Vec<usize> = chars
        .iter()
        .enumerate()
        .filter(|(_, c)| lexicon::DRAMATIC_VOWELS.contains(**c))
        .map(|(i, _)| i)
        .collect();

    if positions.is_empty() {
        return expr.to_string();
    }

    positions.sort_by_key(|&i| {
        lexicon::DRAMATIC_VOWELS
            .find(chars[i])
            .unwrap_or(usize::MAX)
    });

    let top_n = positions.len().div_ceil(2);
    let mut rng = rand::thread_rng();
    let pos = positions[rng.gen_range(0..top_n)];
    let vowel = chars[pos];

    let mut result = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if i == pos {
            for _ in 0..repeats {
                result.push(vowel);
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub fn inject_mat(
    text: &str,
    prob: f64,
    stretch_prob: f64,
    diversifier: &mut WordDiversifier,
    bias: Option<WordBias>,
) -> String {
    if text.is_empty() || prob <= 0.0 {
        return text.to_string();
    }

    let bw: Option<&BiasWordSet> = bias
        .filter(|b| *b != WordBias::Neutral)
        .map(|b| lexicon::bias_words(b));

    let mut rng = rand::thread_rng();

    let sentences: Vec<&str> = split_sentences(text);

    sentences
        .iter()
        .map(|sentence| {
            let mut s = sentence.to_string();
            let scores = score_placement(&s);

            if rng.gen::<f64>() < prob * scores.prefix {
                let pool = bw.map(|b| b.prefix).unwrap_or(lexicon::MAT_PREFIX);
                s = format!("{} {s}", pick_mat(pool, stretch_prob, diversifier));
            }

            let clauses: Vec<&str> = s.split(',').collect();
            let mutated: Vec<String> = clauses
                .iter()
                .enumerate()
                .map(|(i, clause)| {
                    if i == 0 {
                        return clause.to_string();
                    }
                    if rng.gen::<f64>() < prob * scores.mid {
                        let pool = bw.map(|b| b.standalone).unwrap_or(lexicon::MAT_INTERJECT);
                        format!(" {},{}", pick_mat(pool, stretch_prob, diversifier), clause)
                    } else {
                        clause.to_string()
                    }
                })
                .collect();
            s = mutated.join(",");

            if rng.gen::<f64>() < prob * scores.suffix {
                let pool = bw.map(|b| b.suffix).unwrap_or(lexicon::MAT_SUFFIX);
                let (stripped, punct) = strip_trailing_punct(&s);
                s = format!(
                    "{stripped}{}{punct}",
                    pick_mat(pool, stretch_prob, diversifier)
                );
            }

            s
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn inject_interject(
    text: &str,
    prob: f64,
    diversifier: &mut WordDiversifier,
    bias: Option<WordBias>,
) -> String {
    if text.is_empty() || prob <= 0.0 {
        return text.to_string();
    }

    let bw: Option<&BiasWordSet> = bias
        .filter(|b| *b != WordBias::Neutral)
        .map(|b| lexicon::bias_words(b));

    let mut rng = rand::thread_rng();
    let sentences: Vec<&str> = split_sentences(text);

    sentences
        .iter()
        .map(|sentence| {
            let mut s = sentence.to_string();
            let scores = score_placement(&s);

            if rng.gen::<f64>() < prob * scores.prefix {
                let pool = bw.map(|b| b.prefix).unwrap_or(lexicon::INTERJECT_PREFIX);
                s = format!("{} {s}", diversifier.pick(pool));
            }

            let clauses: Vec<&str> = s.split(',').collect();
            let mutated: Vec<String> = clauses
                .iter()
                .enumerate()
                .map(|(i, clause)| {
                    if i == 0 {
                        return clause.to_string();
                    }
                    if rng.gen::<f64>() < prob * scores.mid {
                        let pool = bw.map(|b| b.standalone).unwrap_or(lexicon::INTERJECT_MID);
                        format!(" {},{}", diversifier.pick(pool), clause)
                    } else {
                        clause.to_string()
                    }
                })
                .collect();
            s = mutated.join(",");

            if rng.gen::<f64>() < prob * scores.suffix {
                let pool = bw.map(|b| b.suffix).unwrap_or(lexicon::INTERJECT_SUFFIX);
                let (stripped, punct) = strip_trailing_punct(&s);
                s = format!("{stripped}{}{punct}", diversifier.pick(pool));
            }

            s
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn pick_mat(pool: &[&str], stretch_prob: f64, diversifier: &mut WordDiversifier) -> String {
    let expr = diversifier.pick(pool);
    let mut rng = rand::thread_rng();
    if stretch_prob > 0.0 && rng.gen::<f64>() < stretch_prob {
        let repeats =
            STRETCH_REPEATS_MIN + rng.gen_range(0..=(STRETCH_REPEATS_MAX - STRETCH_REPEATS_MIN));
        stretch_expression(&expr, repeats)
    } else {
        expr
    }
}

fn strip_trailing_punct(s: &str) -> (String, String) {
    let re = Regex::new(r"[.!?]+$").expect("infallible");
    if let Some(m) = re.find(s) {
        (s[..m.start()].to_string(), m.as_str().to_string())
    } else {
        (s.to_string(), ".".to_string())
    }
}

fn split_sentences(text: &str) -> Vec<&str> {
    let re = Regex::new(r"[.!?]\s+").expect("infallible");
    let mut result = Vec::new();
    let mut last = 0;
    for m in re.find_iter(text) {
        let end = m.start() + 1; // include the punctuation
        if end > last {
            result.push(&text[last..end]);
        }
        last = m.end();
    }
    if last < text.len() {
        result.push(&text[last..]);
    }
    if result.is_empty() {
        result.push(text);
    }
    result
}

pub fn apply_glossary(text: &str) -> String {
    let mut s = text.to_string();
    for &(pattern, replacement) in lexicon::IT_GLOSSARY {
        if let Ok(re) = Regex::new(&format!("(?i){pattern}")) {
            s = re.replace_all(&s, replacement).to_string();
        }
    }
    s
}

pub fn scrub_model_artifacts(raw: &str) -> String {
    let mut s = raw.to_string();
    s = Regex::new(r"(?s)<think>.*?</think>")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();
    s = Regex::new(r"(?i)/(no_think|think|system|inst)[^\s]*")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();
    s = Regex::new(r"(?m)^\s*[/\\][a-z_]+\s*$")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();
    s.trim().to_string()
}

pub async fn ollama_translate(
    text: &str,
    url: &str,
    model: &str,
    from: &str,
    to: &str,
) -> Result<String, String> {
    let t = std::time::Instant::now();
    let system = format!(
        "You are a professional {}-to-{} translator for software engineers.\n\
         Rules:\n\
         1. Translate naturally and fluently.\n\
         2. Keep IT terms as Russian loanwords: deploy→деплоить, commit→коммит, \
            branch→ветка, merge→мержить, pull request→пулл-реквест, docker→докер.\n\
         3. Return ONLY the translation. No explanations, no quotes, no comments.",
        from.to_uppercase(),
        to.to_uppercase()
    );

    let body = serde_json::json!({
        "model": model,
        "stream": false,
        "think": false,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": text },
        ],
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{url}/api/chat"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Ollama HTTP {}", resp.status()));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let raw = data["message"]["content"].as_str().unwrap_or(text);
    let cleaned = scrub_model_artifacts(raw);
    tracing::info!(
        ollama_ms = t.elapsed().as_millis() as u64,
        model,
        "translate: ollama done"
    );
    Ok(if cleaned.is_empty() {
        text.to_string()
    } else {
        cleaned
    })
}

/// Translate using the NLLB-200-distilled-600M endpoint on Modal.
///
/// Falls back to the original text on any error so the pipeline never stalls.
pub async fn nllb_translate(text: &str, url: &str, src: &str, tgt: &str) -> Result<String, String> {
    let t = std::time::Instant::now();
    let body = serde_json::json!({
        "text": text,
        "src_lang": src,
        "tgt_lang": tgt,
    });
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("NLLB request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("NLLB HTTP {}", resp.status()));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let result = data["text"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| text.to_owned());

    tracing::info!(
        nllb_ms = t.elapsed().as_millis() as u64,
        "translate: nllb done"
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_question_suppresses_suffix() {
        let s = score_placement("Как дела?");
        assert_eq!(s.suffix, 0.0);
    }

    #[test]
    fn placement_statement_has_suffix() {
        let s = score_placement("Деплой прошёл успешно.");
        assert!(s.suffix > 0.0);
    }

    #[test]
    fn placement_long_sentence_boosts_suffix() {
        let short = score_placement("Привет.");
        let long = score_placement(
            "Деплой прошёл успешно и все тесты прошли и сервер работает нормально.",
        );
        assert!(long.suffix >= short.suffix);
    }

    #[test]
    fn placement_single_clause_has_zero_mid() {
        let s = score_placement("Привет сталкер.");
        assert_eq!(s.mid, 0.0);
    }

    #[test]
    fn placement_multi_clause_has_nonzero_mid() {
        let s = score_placement("Привет, сталкер, как дела.");
        assert!(s.mid > 0.0);
    }

    #[test]
    fn placement_tech_boosts_prefix() {
        let plain = score_placement("Привет.");
        let tech = score_placement("Деплой сломал API.");
        assert!(tech.prefix > plain.prefix);
    }

    #[test]
    fn placement_empty_returns_zeros() {
        let s = score_placement("");
        assert_eq!(s.prefix, 0.0);
        assert_eq!(s.mid, 0.0);
        assert_eq!(s.suffix, 0.0);
    }

    #[test]
    fn placement_scores_within_bounds() {
        for text in &[
            "Short.",
            "Длинное предложение с множеством слов для тестирования границ.",
            "Как дела?",
            "Пайплайн, коммит, деплой, тест, билд, баг, фикс.",
        ] {
            let s = score_placement(text);
            assert!(
                s.prefix >= 0.0 && s.prefix <= 1.0,
                "prefix out of bounds for {text}"
            );
            assert!(s.mid >= 0.0 && s.mid <= 1.0, "mid out of bounds for {text}");
            assert!(
                s.suffix >= 0.0 && s.suffix <= 1.0,
                "suffix out of bounds for {text}"
            );
        }
    }

    #[test]
    fn diversifier_picks_from_array() {
        let mut d = WordDiversifier::new();
        let pool = &["блядь", "сука", "ёпта"];
        let w = d.pick(pool);
        assert!(pool.contains(&w.as_str()));
    }

    #[test]
    fn diversifier_tracks_counts() {
        let mut d = WordDiversifier::new();
        let pool = &["a", "b"];
        for _ in 0..20 {
            d.pick(pool);
        }
        assert!(d.get_count("a") > 0);
        assert!(d.get_count("b") > 0);
    }

    #[test]
    fn diversifier_distributes_picks() {
        let mut d = WordDiversifier::new();
        let pool = &["a", "b", "c"];
        for _ in 0..30 {
            d.pick(pool);
        }
        for w in pool {
            assert!(d.get_count(w) >= 3, "{w} underrepresented");
        }
    }

    #[test]
    fn diversifier_reset_clears() {
        let mut d = WordDiversifier::new();
        d.pick(&["x"]);
        assert_eq!(d.get_count("x"), 1);
        d.reset();
        assert_eq!(d.get_count("x"), 0);
    }

    #[test]
    fn stretch_adds_vowel_repeats() {
        let stretched = stretch_expression("блядь", 3);
        assert!(stretched.len() > "блядь".len());
        assert!(stretched.chars().count() > "блядь".chars().count());
    }

    #[test]
    fn stretch_no_vowels_unchanged() {
        assert_eq!(stretch_expression("хм", 3), "хм");
    }

    #[test]
    fn inject_mat_empty_returns_empty() {
        let mut d = WordDiversifier::new();
        assert_eq!(inject_mat("", 1.0, 0.0, &mut d, None), "");
    }

    #[test]
    fn inject_mat_zero_prob_unchanged() {
        let mut d = WordDiversifier::new();
        let text = "Привет, сталкер.";
        assert_eq!(inject_mat(text, 0.0, 0.0, &mut d, None), text);
    }

    #[test]
    fn inject_mat_high_prob_modifies() {
        let mut d = WordDiversifier::new();
        let text = "Привет, сталкер. Как дела, браток.";
        let mut changed = false;
        for _ in 0..20 {
            d.reset();
            if inject_mat(text, 1.0, 0.0, &mut d, None) != text {
                changed = true;
                break;
            }
        }
        assert!(changed, "mat injection should modify text at prob=1.0");
    }

    #[test]
    fn inject_interject_empty_returns_empty() {
        let mut d = WordDiversifier::new();
        assert_eq!(inject_interject("", 1.0, &mut d, None), "");
    }

    #[test]
    fn inject_interject_high_prob_modifies() {
        let mut d = WordDiversifier::new();
        let text = "Привет, сталкер. Как дела.";
        let mut changed = false;
        for _ in 0..20 {
            d.reset();
            if inject_interject(text, 1.0, &mut d, None) != text {
                changed = true;
                break;
            }
        }
        assert!(
            changed,
            "interject injection should modify text at prob=1.0"
        );
    }

    #[test]
    fn glossary_replaces_it_terms() {
        let result = apply_glossary("Deploy the service and commit the changes");
        assert!(result.contains("деплой"));
        assert!(result.contains("коммит"));
    }

    #[test]
    fn glossary_case_insensitive() {
        assert!(apply_glossary("DEPLOY").contains("деплой"));
    }

    #[test]
    fn glossary_preserves_non_it_text() {
        let text = "Привет, как дела?";
        assert_eq!(apply_glossary(text), text);
    }

    #[test]
    fn scrub_removes_think_blocks() {
        let raw = "Hello <think>internal reasoning</think> world";
        assert_eq!(scrub_model_artifacts(raw), "Hello  world");
    }

    #[test]
    fn scrub_removes_control_tokens() {
        let raw = "Привет /no_think мир";
        assert!(!scrub_model_artifacts(raw).contains("/no_think"));
    }

    #[test]
    fn scrub_clean_text_unchanged() {
        assert_eq!(scrub_model_artifacts("Чистый текст."), "Чистый текст.");
    }
}
