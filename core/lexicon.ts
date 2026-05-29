/**
 * Foni Russian Mat & Prison Jargon Lexicon
 *
 * Sources:
 *   [1] russki-mat.net — historical воровской жаргон dictionaries 1859–1927
 *   [2] vgulage.name — GULAG memoir jargon list
 *   [3] ru.wiktionary.org/wiki/Приложение:Уголовный_жаргон
 *   [4] "Толковый словарь уголовных жаргонов", Москва, 1991
 *   [5] "Словарь русского арго", Владимир Елистратов
 *   [6] "Язык, который ненавидит", Сергей Снегов
 *   [7] Friedman (1979) — Russian obscenity pragmatics paper
 *
 * Pragmatics research finding:
 *   блядь/сука are most naturally sentence-FINAL as general emphasizers.
 *   Prison/street jargon injections work mid-sentence and as standalone exclamations.
 *   Overuse breaks immersion — WordDiversifier handles frequency management.
 */

// ─── Category types ───────────────────────────────────────────────────────────

export interface LexiconEntry {
  /** The word/phrase itself */
  word:      string;
  /** Approximate transliteration for non-Russian readers */
  translit:  string;
  /** English gloss */
  gloss:     string;
  /** Where it sounds natural */
  positions: Array<"prefix" | "mid" | "suffix" | "standalone">;
  /** Intensity: 1=mild, 2=medium, 3=strong mat */
  heat:      1 | 2 | 3;
  /** Source register */
  register:  "mat" | "prison" | "street" | "mild";
}

// ─── Mat — core profanity ─────────────────────────────────────────────────────

export const MAT_LEXICON: LexiconEntry[] = [
  // ── The universals ─────────────────────────────────────────────────────────
  { word: "блядь",         translit: "blyad'",        gloss: "universal filler/emphasizer",         positions: ["suffix", "standalone"],         heat: 3, register: "mat"    },
  { word: "бля",           translit: "blya",           gloss: "clipped блядь, softer",               positions: ["mid", "suffix", "standalone"],  heat: 2, register: "mat"    },
  { word: "блять",         translit: "blyat'",         gloss: "alternate spelling, same force",      positions: ["suffix", "standalone"],         heat: 3, register: "mat"    },
  { word: "сука",          translit: "suka",           gloss: "bitch — pure intensifier in use",     positions: ["suffix", "standalone"],         heat: 3, register: "mat"    },
  { word: "пиздец",        translit: "pizdets",        gloss: "it's fucked / doom / kaput",          positions: ["suffix", "standalone"],         heat: 3, register: "mat"    },
  { word: "хуй",           translit: "khuy",           gloss: "dick — used dismissively mid-clause", positions: ["mid", "standalone"],            heat: 3, register: "mat"    },
  { word: "хуйня",         translit: "khuinya",        gloss: "bullshit / nonsense / crap",          positions: ["mid", "suffix", "standalone"],  heat: 2, register: "mat"    },
  { word: "ёпта",          translit: "yopta",          gloss: "soft exclamation, like 'damn it'",    positions: ["suffix", "standalone"],         heat: 2, register: "mat"    },
  { word: "ёбаный",        translit: "yobany",         gloss: "fucking (adjective)",                 positions: ["prefix"],                       heat: 3, register: "mat"    },
  { word: "нихуя себе",    translit: "nikhuya sebe",   gloss: "holy shit / no fucking way",          positions: ["standalone", "prefix"],         heat: 2, register: "mat"    },
  { word: "заебись",       translit: "zaebis'",        gloss: "fucking great! (POSITIVE)",           positions: ["suffix", "standalone"],         heat: 2, register: "mat"    },
  { word: "пиздить",       translit: "pizdit'",        gloss: "to steal / to beat / to lie",         positions: ["mid"],                          heat: 3, register: "mat"    },
  { word: "охуеть",        translit: "okhuyet'",       gloss: "to be amazed/shocked/outraged",       positions: ["standalone"],                   heat: 3, register: "mat"    },
  { word: "ёб твою мать",  translit: "yob tvoyu mat'", gloss: "classic — motherfucker",              positions: ["prefix", "standalone"],         heat: 3, register: "mat"    },
  { word: "мать твою",     translit: "mat' tvoyu",     gloss: "shortened motherfucker",              positions: ["prefix", "standalone"],         heat: 3, register: "mat"    },
  { word: "твою мать",     translit: "tvoyu mat'",     gloss: "damn it / motherfucker",              positions: ["suffix", "standalone"],         heat: 2, register: "mat"    },
  { word: "ёкарный бабай", translit: "yokarny babay",  gloss: "colorful euphemism for strong mat",  positions: ["standalone", "prefix"],         heat: 1, register: "mild"   },
  { word: "какого хуя",    translit: "kakogo khuya",   gloss: "what the fuck (interrogative)",       positions: ["prefix", "standalone"],         heat: 3, register: "mat"    },
];

// ─── Prison jargon (Феня / блатная музыка) ───────────────────────────────────

export const FENYA_LEXICON: LexiconEntry[] = [
  // ── Exclamations ───────────────────────────────────────────────────────────
  { word: "Опа!",          translit: "Opa!",          gloss: "surprise/admiration — street/prison", positions: ["standalone", "prefix"],         heat: 1, register: "street" },
  { word: "Оп-па!",        translit: "Op-pa!",        gloss: "emphatic Опа",                        positions: ["standalone"],                   heat: 1, register: "street" },
  { word: "Атас!",         translit: "Atas!",         gloss: "watch out! danger! lookout call",     positions: ["standalone"],                   heat: 1, register: "prison" },
  { word: "Шухер!",        translit: "Shukher!",      gloss: "police coming! scatter! (lookout)",   positions: ["standalone"],                   heat: 1, register: "prison" },

  // ── People / roles ─────────────────────────────────────────────────────────
  { word: "братан",        translit: "bratan",        gloss: "brother, close friend",               positions: ["prefix", "standalone"],         heat: 1, register: "street" },
  { word: "кент",          translit: "kent",          gloss: "close friend (феня)",                 positions: ["prefix", "standalone"],         heat: 1, register: "prison" },
  { word: "фраер",         translit: "fraer",         gloss: "sucker / outsider / civilian",        positions: ["mid", "standalone"],            heat: 1, register: "prison" },
  { word: "лох",           translit: "lokh",          gloss: "sucker / easy mark",                  positions: ["mid", "standalone"],            heat: 1, register: "street" },
  { word: "мусор",         translit: "musor",         gloss: "cop (literally 'garbage')",           positions: ["mid", "standalone"],            heat: 2, register: "prison" },
  { word: "мент",          translit: "ment",          gloss: "cop / pig",                           positions: ["mid", "standalone"],            heat: 2, register: "street" },
  { word: "шестёрка",      translit: "shestyorka",    gloss: "lackey / errand boy for criminals",   positions: ["mid", "standalone"],            heat: 1, register: "prison" },
  { word: "зэк",           translit: "zek",           gloss: "prisoner / convict (з/к)",            positions: ["mid", "standalone"],            heat: 1, register: "prison" },
  { word: "пацан",         translit: "patsan",        gloss: "lad / guy (respect term)",            positions: ["prefix", "standalone"],         heat: 1, register: "street" },

  // ── Concepts ────────────────────────────────────────────────────────────────
  { word: "беспредел",     translit: "bespredel",     gloss: "outrage / lawlessness / beyond all",  positions: ["mid", "suffix"],                heat: 2, register: "prison" },
  { word: "понятия",       translit: "ponyatiya",     gloss: "the code / criminal rules",           positions: ["mid"],                          heat: 1, register: "prison" },
  { word: "по понятиям",   translit: "po ponyatiyam", gloss: "by the criminal code",                positions: ["mid", "suffix"],                heat: 1, register: "prison" },
  { word: "разборки",      translit: "razborki",      gloss: "showdown / settling of scores",       positions: ["mid", "suffix"],                heat: 2, register: "street" },
  { word: "крыша",         translit: "krysha",        gloss: "protection racket (literally 'roof')", positions: ["mid"],                         heat: 1, register: "street" },
  { word: "общак",         translit: "obshchak",      gloss: "communal criminal fund",              positions: ["mid"],                          heat: 1, register: "prison" },
  { word: "малява",        translit: "malyava",       gloss: "secret note passed in prison",        positions: ["mid"],                          heat: 1, register: "prison" },
  { word: "прогон",        translit: "progon",        gloss: "bullshit story / lie",                positions: ["mid", "suffix"],                heat: 1, register: "prison" },
  { word: "порожняк",      translit: "porozhnyak",    gloss: "empty talk / nonsense",               positions: ["mid", "suffix"],                heat: 1, register: "prison" },
  { word: "хата",          translit: "khata",         gloss: "cell / room (literally 'hut')",       positions: ["mid"],                          heat: 1, register: "prison" },

  // ── Actions ─────────────────────────────────────────────────────────────────
  { word: "ботать по фене", translit: "botat' po fene", gloss: "to speak in criminal argot",        positions: ["mid"],                          heat: 1, register: "prison" },
  { word: "прогнать",       translit: "prognat'",      gloss: "to cheat / scam / talk nonsense",   positions: ["mid"],                          heat: 1, register: "prison" },
];

// ─── Street / internet slang (colorful but not hard mat) ─────────────────────

export const STREET_LEXICON: LexiconEntry[] = [
  { word: "капец",         translit: "kapets",        gloss: "kaput / done for (mild пиздец)",      positions: ["suffix", "standalone"],         heat: 1, register: "street" },
  { word: "кирдык",        translit: "kirdyk",        gloss: "it's over / done",                    positions: ["suffix", "standalone"],         heat: 1, register: "street" },
  { word: "кранты",        translit: "kranty",        gloss: "it's over / that's it",               positions: ["suffix", "standalone"],         heat: 1, register: "street" },
  { word: "ёшкин кот",     translit: "yoshkin kot",   gloss: "holy moly (euphemism)",               positions: ["standalone"],                   heat: 1, register: "mild"   },
  { word: "ёлки-палки",    translit: "yolki-palki",   gloss: "gosh darn (euphemism)",               positions: ["standalone"],                   heat: 1, register: "mild"   },
  { word: "ядрёна мать",   translit: "yadryona mat'", gloss: "wow/damn (old colorful exclamation)", positions: ["standalone", "prefix"],         heat: 2, register: "street" },
  { word: "мля",           translit: "mlya",          gloss: "softened блядь",                      positions: ["suffix", "standalone"],         heat: 1, register: "mild"   },
  { word: "вот это да",    translit: "vot eto da",    gloss: "wow / no way!",                       positions: ["standalone"],                   heat: 1, register: "street" },
  { word: "вот это поворот", translit: "vot eto povorot", gloss: "what a twist! (meme phrase)",    positions: ["standalone", "suffix"],         heat: 1, register: "street" },
  { word: "ну ты даёшь",   translit: "nu ty dayosh'", gloss: "you're something else / wow",         positions: ["standalone"],                   heat: 1, register: "street" },
  { word: "ёкарный бабай", translit: "yokarny babay", gloss: "colorful non-mat exclamation",        positions: ["standalone"],                   heat: 1, register: "mild"   },
];

// ─── Iconic phrases (full expressions, injection-ready) ───────────────────────
//
// These are complete expressions, not single words — inject as standalone
// between sentences or as sentence-final beats.

export const ICONIC_PHRASES: Array<{ phrase: string; translit: string; gloss: string; heat: 1|2|3 }> = [
  { phrase: "Нихуя ты охуел",      translit: "Nikhuya ty okhuvel",      gloss: "you've completely lost it",           heat: 3 },
  { phrase: "Ты вообще в курсе?",   translit: "Ty voobshche v kurse?",   gloss: "are you even aware of this?",         heat: 1 },
  { phrase: "Ёпт, ну ты даёшь",    translit: "Ypt, nu ty dayosh'",      gloss: "damn, you're really something",       heat: 2 },
  { phrase: "Ёбаный в рот, ну и ну", translit: "Yobany v rot, nu i nu", gloss: "fucking hell, well well",             heat: 3 },
  { phrase: "Да ну нахуй",          translit: "Da nu nakhuy",           gloss: "no fucking way / get outta here",     heat: 3 },
  { phrase: "Какого хуя вообще",    translit: "Kakogo khuya voobshche", gloss: "what the actual fuck",                heat: 3 },
  { phrase: "Пиздец котёнку",       translit: "Pizdets kotyonku",       gloss: "the kitten is fucked — it's over",    heat: 3 },
  { phrase: "Ну и дела",            translit: "Nu i dela",              gloss: "well I never / what's going on",      heat: 1 },
  { phrase: "Это вообще законно?",  translit: "Eto voobshche zakonno?", gloss: "is this even legal?",                 heat: 1 },
  { phrase: "Всё, приехали",        translit: "Vsyo, priyekhali",       gloss: "that's it, we've arrived (at the end)", heat: 1 },
  { phrase: "Капец, ну и ну",       translit: "Kapets, nu i nu",        gloss: "well that's done for",                heat: 1 },
  { phrase: "Ты серьёзно сейчас?",  translit: "Ty seryozno seychas?",   gloss: "are you serious right now?",          heat: 1 },
  { phrase: "Беспредел полный",     translit: "Bespredel polny",        gloss: "total outrage / utter lawlessness",   heat: 2 },
  { phrase: "По понятиям не канает", translit: "Po ponyatiyam ne kanаyet", gloss: "doesn't fly by the code",          heat: 2 },
];

// ─── Resources for more ───────────────────────────────────────────────────────

export const LEXICON_RESOURCES = [
  {
    name:   "russki-mat.net",
    url:    "https://www.russki-mat.net",
    desc:   "Historical criminal jargon dictionaries 1859–1927. Full vorovskoy zhargon from primary sources.",
    lang:   "ru",
  },
  {
    name:   "vgulage.name — Список жаргонных слов",
    url:    "https://vgulage.name/chapters/spisok-zhargonnyh-slov",
    desc:   "GULAG memoir archive. Authentic prisoner vocabulary from first-hand accounts.",
    lang:   "ru",
  },
  {
    name:   "ru.wiktionary.org — Уголовный жаргон",
    url:    "https://ru.wiktionary.org/wiki/Приложение:Уголовный_жаргон",
    desc:   "Wiktionary appendix of Russian criminal jargon. A–Я, constantly updated.",
    lang:   "ru",
  },
  {
    name:   "Толковый словарь уголовных жаргонов (1991)",
    url:    "https://imwerden.de/pdf/tolkovy_slovar_ugolovnykh_zhargonov_1991__ocr.pdf",
    desc:   "Full academic dictionary of criminal jargon — PDF, OCR. Gold standard reference.",
    lang:   "ru",
  },
  {
    name:   "Словарь русского арго — Елистратов",
    url:    "https://lib.ru/NEWPROZA/SIDOROV_A/slowari.txt",
    desc:   "Vladimir Elistratov's Argo dictionary. Includes mat and blat vocabulary with etymology.",
    lang:   "ru",
  },
  {
    name:   "Феня — Wikipedia RU",
    url:    "https://ru.wikipedia.org/wiki/Феня",
    desc:   "Overview of fenya origin, evolution from Ofen language, modern usage.",
    lang:   "ru",
  },
  {
    name:   "lifehacker.ru — 13 тюремных слов в обычной речи",
    url:    "https://lifehacker.ru/privychnyj-tyuremnyj-zhargon",
    desc:   "Prison words that entered everyday Russian speech. Good for natural injection targets.",
    lang:   "ru",
  },
] as const;

// ─── Convenience re-exports grouped by use ───────────────────────────────────

/** All suffix-appropriate mat for sentence-final injection */
export const SUFFIX_MAT = MAT_LEXICON
  .filter(e => e.positions.includes("suffix"))
  .map(e => e.word);

/** All prefix-appropriate expressions */
export const PREFIX_EXPRESSIONS = [
  ...MAT_LEXICON.filter(e => e.positions.includes("prefix")).map(e => e.word),
  ...FENYA_LEXICON.filter(e => e.positions.includes("prefix")).map(e => e.word),
];

/** Standalone exclamations (between sentences, as fillers) */
export const STANDALONE_EXCLAMATIONS = [
  ...MAT_LEXICON.filter(e => e.positions.includes("standalone")).map(e => e.word),
  ...FENYA_LEXICON.filter(e => e.positions.includes("standalone")).map(e => e.word),
  ...STREET_LEXICON.filter(e => e.positions.includes("standalone")).map(e => e.word),
];

/** Prison-flavour mid-sentence drops */
export const PRISON_MID = FENYA_LEXICON
  .filter(e => e.positions.includes("mid"))
  .map(e => e.word);
