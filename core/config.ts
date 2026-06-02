// ─── Foni domain config ───────────────────────────────────────────────────────
//
// Zero pi dependencies. Pi extension reads and writes this object;
// FoniEngine owns it. Future: serialise to disk for persistence.

export interface FoniConfig {
  // Core
  enabled:     boolean;
  backendPref: "auto" | "espeak" | "say";
  voice:       string;
  speed:       number;
  inputLang:   "en" | "ru";
  outputLang:  "en" | "ru";

  // Backend URLs

  // Mat — Russian profanity injection
  matEnabled:      boolean;
  matProb:         number;
  matStretch:      number;
  matCooldownMs:   number; // min ms between injections across chunks

  interjectEnabled:    boolean;
  interjectProb:       number;
  interjectCooldownMs: number;

  // RVC — Sidorovich voice
  rvcEnabled: boolean;
  rvcUrl:     string;
  rvcModel:   string;

  // Prosody — SSML break/rate/pitch annotation
  prosodyEnabled: boolean;

  // Breath injection — splice breath sounds at silence gaps
  breathEnabled: boolean;
}

export const DEFAULT_CONFIG: FoniConfig = {
  enabled:     true,
  backendPref: "espeak",
  voice:       "en_0",
  speed:       1.0,
  inputLang:   "en",
  outputLang:  "ru",


  matEnabled:      true,
  matProb:         0.35,
  matStretch:      0.5,
  matCooldownMs:   20_000, // at most once per 20s

  interjectEnabled:    true,
  interjectProb:       0.25,
  interjectCooldownMs: 12_000, // at most once per 12s

  rvcEnabled: true,
  rvcUrl:     "http://localhost:5050",
  rvcModel:   "sidorovich",

  prosodyEnabled: true,
  breathEnabled:  true,
};

// ─── Prewarm phrases ──────────────────────────────────────────────────────────
//
// Synthesised in parallel on session_start so the AudioLRU is warm before
// Claude speaks. Only activated when outputLang === "ru" and rvcEnabled.

export const PREWARM_RU: readonly string[] = [
  // Acknowledgements
  "Да.", "Нет.", "Хорошо.", "Понял.", "Окей.", "Готово.",
  // Common short responses
  "Сейчас.", "Подожди.", "Конечно.",
  // Mat vocabulary (standalone high-frequency)
  "Блядь.", "Пиздец.", "Ёпта.", "Сука.",
  // Interjections
  "Ого!", "Ах!", "Ух!", "Эх.",
];

// ─── Filler phrases ───────────────────────────────────────────────────────────
//
// Short Russian hesitation sounds synthesised via RVC during prewarm.
// Played between agent_start and first onDelta to fill the thinking gap.
// Ordered from subtle humming → verbal fillers → self-talk.

export const FILLER_PHRASES: readonly string[] = [
  // Hesitation sounds — pure vocalisations
  "Мм...",
  "Хм...",
  "Эм...",
  // Verbal fillers — floor-holding words
  "Ну...",
  "Так...",
  "Значит...",
  "Это...",
  // Self-talk — thinking out loud
  "Так, так, так...",
  "Ну, значит...",
  "Сейчас, сейчас...",
  "Дай подумать...",
];
