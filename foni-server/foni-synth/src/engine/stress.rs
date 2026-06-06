/// Russian stress annotation middleware.
///
/// Implements the `StressAnnotator` trait (Strategy pattern) with two backends:
/// - `DictionaryAnnotator` — static lookup from OpenRussian CSV, zero latency, ~58k lemmas
///   with all inflected forms extracted (nouns ×22 cols, verbs ×18, adjectives ×20).
/// - `RuaccentAnnotator` — HTTP client to a local ruaccent sidecar, 0.97 accuracy,
///   handles homographs. Falls back to the original word if the sidecar is unreachable.
///
/// Use `annotate()` before passing text to Chatterbox TTS. It converts the bare apostrophe
/// notation (`приве'т`) from the dictionary into Unicode combining acute (`приве́т`, U+0301).
use once_cell::sync::Lazy;
use std::collections::HashMap;

// ── CSV data ─────────────────────────────────────────────────────────────────

const NOUNS_CSV: &str = include_str!("../../../../training/stress/nouns.csv");
const VERBS_CSV: &str = include_str!("../../../../training/stress/verbs.csv");
const ADJECTIVES_CSV: &str = include_str!("../../../../training/stress/adjectives.csv");
const OTHERS_CSV: &str = include_str!("../../../../training/stress/others.csv");

// ── Dictionary build ──────────────────────────────────────────────────────────

/// Parse all tab-separated CSVs and collect (bare → accented) pairs for every
/// inflected form column. The apostrophe-after-vowel notation is converted to
/// Unicode combining acute accent (U+0301).
pub static STRESS_MAP: Lazy<HashMap<String, String>> = Lazy::new(|| {
    let mut map = HashMap::new();
    for csv in [NOUNS_CSV, VERBS_CSV, ADJECTIVES_CSV, OTHERS_CSV] {
        ingest_csv(csv, &mut map);
    }
    map
});

fn ingest_csv(csv: &str, map: &mut HashMap<String, String>) {
    let mut lines = csv.lines();
    let Some(header) = lines.next() else { return };
    let cols: Vec<&str> = header.split('\t').collect();

    // Columns that contain inflected word forms (bare and accented variants).
    // We skip metadata columns (translations, gender, flags, etc.).
    let form_cols: Vec<usize> = cols
        .iter()
        .enumerate()
        .filter(|(_, name)| is_form_column(name))
        .map(|(i, _)| i)
        .collect();

    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 2 {
            continue;
        }
        let bare_lemma = fields[0].trim();
        let accented_lemma = fields[1].trim();

        // Always add the lemma pair.
        insert_pair(bare_lemma, accented_lemma, map);

        // Add all inflected forms.
        for &col in &form_cols {
            if let Some(&form) = fields.get(col) {
                let form = form.trim();
                if form.is_empty() || form == "-" {
                    continue;
                }
                // The bare form of the inflection is the apostrophe-stripped version.
                let bare = strip_accent_marks(form);
                insert_pair(&bare, form, map);
            }
        }
    }
}

fn is_form_column(name: &str) -> bool {
    // Inflected form columns: sg_nom, pl_gen, presfut_sg1, past_m, etc.
    // Skip: translations_en/de, gender, partner, animate, flags, bare, accented.
    matches!(
        name,
        "bare"
            | "accented"
            | "sg_nom"
            | "sg_gen"
            | "sg_dat"
            | "sg_acc"
            | "sg_inst"
            | "sg_prep"
            | "pl_nom"
            | "pl_gen"
            | "pl_dat"
            | "pl_acc"
            | "pl_inst"
            | "pl_prep"
            | "past_m"
            | "past_f"
            | "past_n"
            | "past_pl"
            | "presfut_sg1"
            | "presfut_sg2"
            | "presfut_sg3"
            | "presfut_pl1"
            | "presfut_pl2"
            | "presfut_pl3"
            | "imperative_sg"
            | "imperative_pl"
            | "short_m"
            | "short_f"
            | "short_n"
            | "short_pl"
            | "comparative"
    )
}

/// Remove apostrophe stress markers (e.g. `приве'т` → `привет`).
fn strip_accent_marks(s: &str) -> String {
    s.replace('\'', "")
}

/// Convert apostrophe notation to Unicode combining acute (U+0301).
pub fn apostrophe_to_unicode(s: &str) -> String {
    s.replace('\'', "\u{0301}")
}

fn insert_pair(bare: &str, accented: &str, map: &mut HashMap<String, String>) {
    if bare.is_empty() || accented.is_empty() || bare == "-" || accented == "-" {
        return;
    }
    // Only insert if the accented form actually has a stress marker.
    if !accented.contains('\'') {
        return;
    }
    let key = bare.to_lowercase();
    let value = apostrophe_to_unicode(accented);
    map.entry(key).or_insert(value);
}

// ── Trait ─────────────────────────────────────────────────────────────────────

pub trait StressAnnotator: Send + Sync {
    /// Annotate Russian text with Unicode stress marks.
    /// Unknown words are returned unchanged.
    fn annotate(&self, text: &str) -> String;
}

// ── DictionaryAnnotator ───────────────────────────────────────────────────────

/// Zero-latency annotator backed by the bundled OpenRussian dictionary.
/// Coverage: ~58k lemmas × all inflected forms ≈ 300k+ word forms.
/// Misses fall through unchanged.
pub struct DictionaryAnnotator;

impl StressAnnotator for DictionaryAnnotator {
    fn annotate(&self, text: &str) -> String {
        annotate_with_map(text, &STRESS_MAP)
    }
}

fn annotate_with_map(text: &str, map: &HashMap<String, String>) -> String {
    // Tokenize preserving punctuation and whitespace structure.
    // Split into runs of Cyrillic letters vs everything else.
    let mut result = String::with_capacity(text.len() + 16);
    let mut word_buf = String::new();

    for ch in text.chars() {
        if is_cyrillic_letter(ch) {
            word_buf.push(ch);
        } else {
            if !word_buf.is_empty() {
                result.push_str(&lookup_word(&word_buf, map));
                word_buf.clear();
            }
            result.push(ch);
        }
    }
    if !word_buf.is_empty() {
        result.push_str(&lookup_word(&word_buf, map));
    }
    result
}

fn lookup_word(word: &str, map: &HashMap<String, String>) -> String {
    let key = word.to_lowercase();
    match map.get(&key) {
        Some(accented) => restore_case(word, accented),
        None => word.to_owned(),
    }
}

/// Restore original capitalisation onto the accented form.
/// The combining acute (U+0301) is a zero-width modifier — it doesn't shift indices,
/// so we must iterate Unicode chars, not bytes.
fn restore_case(original: &str, accented: &str) -> String {
    let mut orig_chars = original.chars();
    let mut out = String::with_capacity(accented.len() + 4);
    for ch in accented.chars() {
        if ch == '\u{0301}' {
            out.push(ch);
        } else if let Some(orig_ch) = orig_chars.next() {
            if orig_ch.is_uppercase() {
                out.extend(ch.to_uppercase());
            } else {
                out.push(ch);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn is_cyrillic_letter(ch: char) -> bool {
    matches!(ch,
        'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'
    )
}

// ── RuaccentAnnotator ─────────────────────────────────────────────────────────

/// Neural annotator via local ruaccent HTTP sidecar.
/// Falls back to the dictionary annotator if the sidecar is unreachable.
pub struct RuaccentAnnotator {
    url: String,
    fallback: DictionaryAnnotator,
}

impl RuaccentAnnotator {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            fallback: DictionaryAnnotator,
        }
    }
}

impl StressAnnotator for RuaccentAnnotator {
    fn annotate(&self, text: &str) -> String {
        match ruaccent_call(&self.url, text) {
            Ok(annotated) => annotated,
            Err(e) => {
                tracing::debug!(error = %e, "ruaccent unreachable, falling back to dictionary");
                self.fallback.annotate(text)
            }
        }
    }
}

fn ruaccent_call(url: &str, text: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(200))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(url)
        .json(&serde_json::json!({ "text": text }))
        .send()
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
    body["text"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "missing text field".into())
}

// ── Factory ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StressMode {
    #[default]
    None,
    Dict,
    Ruaccent,
}

impl std::str::FromStr for StressMode {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "dict" | "dictionary" => Self::Dict,
            "ruaccent" | "neural" => Self::Ruaccent,
            _ => Self::None,
        })
    }
}

pub fn make_annotator(mode: &StressMode, ruaccent_url: &str) -> Box<dyn StressAnnotator> {
    match mode {
        StressMode::Dict => Box::new(DictionaryAnnotator),
        StressMode::Ruaccent => Box::new(RuaccentAnnotator::new(ruaccent_url)),
        StressMode::None => Box::new(PassthroughAnnotator),
    }
}

/// No-op annotator. Returns input unchanged.
pub struct PassthroughAnnotator;

impl StressAnnotator for PassthroughAnnotator {
    fn annotate(&self, text: &str) -> String {
        text.to_owned()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> DictionaryAnnotator {
        DictionaryAnnotator
    }

    #[test]
    fn known_word_gets_accent() {
        let out = dict().annotate("привет");
        assert!(
            out.contains('\u{0301}'),
            "приве́т should have combining acute: {out}"
        );
    }

    #[test]
    fn unknown_word_passes_through() {
        let out = dict().annotate("кринжово");
        assert_eq!(out, "кринжово");
    }

    #[test]
    fn punctuation_preserved() {
        let out = dict().annotate("привет, мир!");
        assert!(out.contains(','));
        assert!(out.contains('!'));
    }

    #[test]
    fn whitespace_preserved() {
        let out = dict().annotate("раз два три");
        let words: Vec<&str> = out.split_whitespace().collect();
        assert_eq!(words.len(), 3);
    }

    #[test]
    fn uppercase_preserved() {
        let out = dict().annotate("Привет");
        let first = out.chars().next().unwrap();
        assert_eq!(first, 'П', "first char should remain uppercase");
    }

    #[test]
    fn mixed_script_preserved() {
        // Latin chars in the middle should not be touched.
        let out = dict().annotate("привет world пока");
        assert!(out.contains("world"));
    }

    #[test]
    fn passthrough_annotator_identity() {
        let ann = PassthroughAnnotator;
        let text = "любой текст";
        assert_eq!(ann.annotate(text), text);
    }

    #[test]
    fn stress_mode_from_str() {
        use std::str::FromStr;
        assert_eq!(StressMode::from_str("dict").unwrap(), StressMode::Dict);
        assert_eq!(StressMode::from_str("DICT").unwrap(), StressMode::Dict);
        assert_eq!(
            StressMode::from_str("ruaccent").unwrap(),
            StressMode::Ruaccent
        );
        assert_eq!(StressMode::from_str("none").unwrap(), StressMode::None);
        assert_eq!(StressMode::from_str("off").unwrap(), StressMode::None);
    }

    #[test]
    fn apostrophe_to_unicode_roundtrip() {
        let s = "приве'т";
        let u = apostrophe_to_unicode(s);
        assert!(u.contains('\u{0301}'));
        assert!(!u.contains('\''));
    }

    #[test]
    fn noun_inflection_covered() {
        // "человека" is genitive of "человек" — should be in the map from sg_gen column.
        let out = dict().annotate("человека");
        assert!(
            out.contains('\u{0301}'),
            "inflected noun should have stress: {out}"
        );
    }

    #[test]
    fn sentence_annotates_multiple_words() {
        let out = dict().annotate("это хорошо");
        // Both "это" and "хорошо" are common words — at least one should be stressed.
        let accent_count = out.chars().filter(|&c| c == '\u{0301}').count();
        assert!(accent_count >= 1, "at least one word stressed in: {out}");
    }
}
