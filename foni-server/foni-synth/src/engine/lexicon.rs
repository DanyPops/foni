use std::sync::OnceLock;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CharacterSeed {
    pub persona: String,
    pub expressions: Vec<String>,
}

#[derive(serde::Deserialize)]
struct LexiconFile {
    character_seed: CharacterSeed,
}

static LEXICON_YAML: &str = include_str!("../../lexicon.yaml");
static CHARACTER_SEED: OnceLock<CharacterSeed> = OnceLock::new();

/// Returns the compiled-in character seed, overridable via `FONI_LEXICON_PATH`.
pub fn character_seed() -> &'static CharacterSeed {
    CHARACTER_SEED.get_or_init(|| {
        let yaml = std::env::var("FONI_LEXICON_PATH")
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(|| LEXICON_YAML.to_string());
        let file: LexiconFile = serde_yaml::from_str(&yaml)
            .expect("invalid lexicon.yaml — check character_seed section");
        file.character_seed
    })
}

pub const MAT_INTERJECT: &[&str] = &[
    "блядь",
    "сука",
    "ёб твою мать",
    "хуй",
    "пиздец",
    "ёпта",
    "нихуя себе",
    "блин",
];

pub const MAT_PREFIX: &[&str] = &[
    "ёбаный в рот,",
    "какого хуя,",
    "мать твою,",
    "ни хуя себе,",
    "чёрт возьми,",
];

pub const MAT_SUFFIX: &[&str] = &[
    ", блядь",
    ", сука",
    ", пиздец",
    ", ёб твою мать",
    ", ёпта",
    ", мать его",
];

pub const INTERJECT_PREFIX: &[&str] = &[
    "Ого!",
    "Ах!",
    "Ух!",
    "Ой!",
    "Эй!",
    "Ба!",
    "Ишь ты!",
    "Ну и ну!",
    "Вот те на!",
    "О-го-го!",
];

pub const INTERJECT_SUFFIX: &[&str] = &[
    ", эх!",
    ", уф!",
    ", ого!",
    ", ай-яй-яй!",
    ", вот как!",
    ", ишь!",
];

pub const INTERJECT_MID: &[&str] = &["ой", "ух", "эх", "ну", "ай"];

pub const DRAMATIC_VOWELS: &str = "АаОоУуЕеЁёИиЭэЮюЯяЫы";

pub struct BiasWordSet {
    pub prefix: &'static [&'static str],
    pub suffix: &'static [&'static str],
    pub standalone: &'static [&'static str],
}

pub const BIAS_AGGRESSIVE: BiasWordSet = BiasWordSet {
    suffix: &[", блядь", ", сука", ", пиздец", ", ёб твою мать"],
    standalone: &["блядь", "сука", "пиздец", "охуеть"],
    prefix: &["Ёбаный в рот,", "Какого хуя,", "Мать твою,"],
};

pub const BIAS_COMMISERATION: BiasWordSet = BiasWordSet {
    suffix: &[", пиздец", ", капец", ", беспредел", ", бля"],
    standalone: &["капец", "кирдык", "пиздец", "ёпта"],
    prefix: &["Ну и дела,", "Беспредел полный,", "Чёрт возьми,"],
};

pub const BIAS_MOCKERY: BiasWordSet = BiasWordSet {
    suffix: &[", ха!", ", нежная душа!", ", цаца!", ", слюнтяй!"],
    standalone: &[
        "Ха!",
        "Нежная душа!",
        "Цаца какая!",
        "Слюнтяй!",
        "Размазня!",
        "Ой, бедняжка!",
    ],
    prefix: &["Ой, ну надо же,", "Смотри-ка,", "Вот те раз,"],
};

pub const BIAS_EXCITEMENT: BiasWordSet = BiasWordSet {
    suffix: &[", нихуя себе!", ", вот это да!", ", нехило!", ", ого!"],
    standalone: &["Нихуя себе!", "Ого!", "Вот это да!", "Охуеть!", "Нехило!"],
    prefix: &["Ого,", "Ну ничего себе,", "Вот это поворот,"],
};

pub const BIAS_NEUTRAL: BiasWordSet = BiasWordSet {
    suffix: &[", блядь", ", сука", ", ёпта", ", бля"],
    standalone: &["блядь", "сука", "ёпта", "пиздец"],
    prefix: &["Ёбаный в рот,", "Мать твою,", "Какого хуя,"],
};

pub fn bias_words(bias: super::emotion::WordBias) -> &'static BiasWordSet {
    use super::emotion::WordBias;
    match bias {
        WordBias::Aggressive => &BIAS_AGGRESSIVE,
        WordBias::Commiseration => &BIAS_COMMISERATION,
        WordBias::Mockery => &BIAS_MOCKERY,
        WordBias::Excitement => &BIAS_EXCITEMENT,
        WordBias::Neutral => &BIAS_NEUTRAL,
    }
}

pub const IT_GLOSSARY: &[(&str, &str)] = &[
    (r"\bpull\s+request(s)?\b", "пуллреквест"),
    (r"\bcommit(s|ted|ting)?\b", "коммит"),
    (r"\bmerge(d|ing)?\b", "мерж"),
    (r"\brebase(d|ing)?\b", "ребейс"),
    (r"\bcheckout\b", "чекаут"),
    (r"\bstash\b", "стеш"),
    (r"\bpush(ed|ing)?\b", "пуш"),
    (r"\bpull(ed|ing)?\b", "пулл"),
    (r"\bfork(ed|ing)?\b", "форк"),
    (r"\bbranch(es)?\b", "ветка"),
    (r"\b[Gg]it\b", "гит"),
    (r"\bdeploy(ment|ed|ing|s)?\b", "деплой"),
    (r"\brollback\b", "роллбек"),
    (r"\bpipeline(s)?\b", "пайплайн"),
    (r"\bstaging\b", "стейджинг"),
    (r"\bprod(uction)?\b", "прод"),
    (r"\bcontainer(s)?\b", "контейнер"),
    (r"\b[Dd]ocker\b", "докер"),
    (r"\b[Kk]ubernetes\b", "кубернетес"),
    (r"\bk8s\b", "кубернетес"),
    (r"\bdebug(ging|ged)?\b", "дебаг"),
    (r"\brefactor(ing|ed)?\b", "рефактор"),
    (r"\bbuild(ing|s)?\b", "билд"),
    (r"\btest(s|ing|ed)?\b", "тест"),
    (r"\bfeature(s)?\b", "фича"),
    (r"\bbug(s|fix)?\b", "баг"),
    (r"\bfix(ed|ing|es)?\b", "фикс"),
    (r"\bcache\b", "кэш"),
    (r"\blog(s|ging)?\b", "лог"),
    (r"\bbackend\b", "бэкенд"),
    (r"\bfrontend\b", "фронтенд"),
    (r"\bserver(s)?\b", "сервер"),
    (r"\bdatabase\b", "база данных"),
    (r"\brelease(s|d)?\b", "релиз"),
    (r"\bcode\s+review\b", "ревью"),
    (r"\breview(ed|ing|s)?\b", "ревью"),
    (r"\bsprint(s)?\b", "спринт"),
    (r"\bstandup\b", "стендап"),
    (r"\bticket(s)?\b", "тикет"),
];
