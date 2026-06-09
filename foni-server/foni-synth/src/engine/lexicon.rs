/// Lexicon — all word pools loaded from `lexicon.yaml` at startup.
///
/// The YAML file is compiled in as the default via `include_str!` and can be
/// overridden at runtime via the `FONI_LEXICON_PATH` environment variable. No
/// recompile needed to edit word lists or IT glossary entries.
///
/// Public interface is identical to the old `const` approach so `translator.rs`
/// needs no changes. Strings are leaked to `'static` once at first access.
use std::sync::OnceLock;

// ── YAML schema ───────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct LexiconFile {
    character_seed: CharacterSeedRaw,
    mat: MatPools,
    interject: InterjectPools,
    dramatic_vowels: String,
    bias: BiasSets,
    it_glossary: Vec<GlossaryEntry>,
}

#[derive(serde::Deserialize)]
struct CharacterSeedRaw {
    persona: String,
    expressions: Vec<String>,
}

#[derive(serde::Deserialize)]
struct MatPools {
    interject: Vec<String>,
    prefix: Vec<String>,
    suffix: Vec<String>,
}

#[derive(serde::Deserialize)]
struct InterjectPools {
    prefix: Vec<String>,
    suffix: Vec<String>,
    mid: Vec<String>,
}

#[derive(serde::Deserialize)]
struct BiasSets {
    aggressive: BiasRaw,
    commiseration: BiasRaw,
    mockery: BiasRaw,
    excitement: BiasRaw,
    neutral: BiasRaw,
}

#[derive(serde::Deserialize)]
struct BiasRaw {
    prefix: Vec<String>,
    suffix: Vec<String>,
    standalone: Vec<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct GlossaryEntry {
    pub(crate) pattern: String,
    pub(crate) replacement: String,
}

// ── Public types ──────────────────────────────────────────────────────────────

/// The character seed used to prompt `ollama_commentary`.
#[derive(Debug, Clone)]
pub struct CharacterSeed {
    pub persona: String,
    pub expressions: Vec<String>,
}

/// Emotion-keyed word set — all fields are `'static` slices for zero-copy access.
pub struct BiasWordSet {
    pub prefix: &'static [&'static str],
    pub suffix: &'static [&'static str],
    pub standalone: &'static [&'static str],
}

// ── Runtime lexicon ───────────────────────────────────────────────────────────

struct Lexicon {
    character_seed: CharacterSeed,

    mat_interject: &'static [&'static str],
    mat_prefix: &'static [&'static str],
    mat_suffix: &'static [&'static str],

    interject_prefix: &'static [&'static str],
    interject_suffix: &'static [&'static str],
    interject_mid: &'static [&'static str],

    dramatic_vowels: &'static str,

    bias_aggressive: BiasWordSet,
    bias_commiseration: BiasWordSet,
    bias_mockery: BiasWordSet,
    bias_excitement: BiasWordSet,
    bias_neutral: BiasWordSet,

    it_glossary: &'static [(&'static str, &'static str)],
}

static LEXICON_YAML: &str = include_str!("../../lexicon.yaml");
static LEXICON: OnceLock<Lexicon> = OnceLock::new();

fn load() -> Lexicon {
    let raw_yaml = std::env::var("FONI_LEXICON_PATH")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| LEXICON_YAML.to_string());

    let file: LexiconFile = serde_yaml::from_str(&raw_yaml).expect("invalid lexicon.yaml");

    Lexicon {
        character_seed: CharacterSeed {
            persona: file.character_seed.persona,
            expressions: file.character_seed.expressions,
        },

        mat_interject: leak_strs(file.mat.interject),
        mat_prefix: leak_strs(file.mat.prefix),
        mat_suffix: leak_strs(file.mat.suffix),

        interject_prefix: leak_strs(file.interject.prefix),
        interject_suffix: leak_strs(file.interject.suffix),
        interject_mid: leak_strs(file.interject.mid),

        dramatic_vowels: leak_str(file.dramatic_vowels),

        bias_aggressive: leak_bias(file.bias.aggressive),
        bias_commiseration: leak_bias(file.bias.commiseration),
        bias_mockery: leak_bias(file.bias.mockery),
        bias_excitement: leak_bias(file.bias.excitement),
        bias_neutral: leak_bias(file.bias.neutral),

        it_glossary: leak_glossary(file.it_glossary),
    }
}

fn lexicon() -> &'static Lexicon {
    LEXICON.get_or_init(load)
}

// ── Leak helpers ──────────────────────────────────────────────────────────────

fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_strs(v: Vec<String>) -> &'static [&'static str] {
    let leaked: Vec<&'static str> = v.into_iter().map(leak_str).collect();
    Box::leak(leaked.into_boxed_slice())
}

fn leak_bias(raw: BiasRaw) -> BiasWordSet {
    BiasWordSet {
        prefix: leak_strs(raw.prefix),
        suffix: leak_strs(raw.suffix),
        standalone: leak_strs(raw.standalone),
    }
}

fn leak_glossary(entries: Vec<GlossaryEntry>) -> &'static [(&'static str, &'static str)] {
    let pairs: Vec<(&'static str, &'static str)> = entries
        .into_iter()
        .map(|e| (leak_str(e.pattern), leak_str(e.replacement)))
        .collect();
    Box::leak(pairs.into_boxed_slice())
}

// ── Public accessors — same names as the old `const` items ───────────────────

pub fn character_seed() -> &'static CharacterSeed {
    &lexicon().character_seed
}

pub fn mat_interject() -> &'static [&'static str] {
    lexicon().mat_interject
}

pub fn mat_prefix() -> &'static [&'static str] {
    lexicon().mat_prefix
}

pub fn mat_suffix() -> &'static [&'static str] {
    lexicon().mat_suffix
}

pub fn interject_prefix() -> &'static [&'static str] {
    lexicon().interject_prefix
}

pub fn interject_suffix() -> &'static [&'static str] {
    lexicon().interject_suffix
}

pub fn interject_mid() -> &'static [&'static str] {
    lexicon().interject_mid
}

pub fn dramatic_vowels() -> &'static str {
    lexicon().dramatic_vowels
}

pub fn bias_words(bias: super::emotion::WordBias) -> &'static BiasWordSet {
    use super::emotion::WordBias;
    let l = lexicon();
    match bias {
        WordBias::Aggressive => &l.bias_aggressive,
        WordBias::Commiseration => &l.bias_commiseration,
        WordBias::Mockery => &l.bias_mockery,
        WordBias::Excitement => &l.bias_excitement,
        WordBias::Neutral => &l.bias_neutral,
    }
}

pub fn it_glossary() -> &'static [(&'static str, &'static str)] {
    lexicon().it_glossary
}
