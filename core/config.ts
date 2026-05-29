// ─── Foni domain config ───────────────────────────────────────────────────────
//
// Zero pi dependencies. Pi extension reads and writes this object;
// FoniEngine owns it. Future: serialise to disk for persistence.

export interface FoniConfig {
  // Core
  enabled:     boolean;
  backendPref: "auto" | "silero" | "kokoro" | "fakeyou" | "espeak" | "say";
  voice:       string;
  speed:       number;
  inputLang:   "en" | "ru";
  outputLang:  "en" | "ru";

  // Backend URLs
  sileroUrl:    string;
  kokoroUrl:    string;
  fakeyouToken: string;
  fakeyouApiKey: string;

  // Mat — Russian profanity injection
  matEnabled: boolean;
  matProb:    number;
  matStretch: number;

  // Interjections — Russian exclamations
  interjectEnabled: boolean;
  interjectProb:    number;

  // RVC — bandit voice
  rvcEnabled: boolean;
  rvcUrl:     string;
  rvcModel:   string;

  // Prosody — SSML break/rate/pitch annotation (FON-TSK-58)
  prosodyEnabled: boolean;

  // Breath injection — splice breath sounds at silence gaps (FON-TSK-59)
  breathEnabled: boolean;
}

export const DEFAULT_CONFIG: FoniConfig = {
  enabled:     true,
  backendPref: "espeak",
  voice:       "en_0",
  speed:       1.15,
  inputLang:   "en",
  outputLang:  "ru",

  sileroUrl:    "http://localhost:8001",
  kokoroUrl:    "http://localhost:8880",
  fakeyouToken: "",
  fakeyouApiKey: "",

  matEnabled: true,
  matProb:    0.35,
  matStretch: 0.5,

  interjectEnabled: true,
  interjectProb:    0.25,

  rvcEnabled: true,
  rvcUrl:     "http://localhost:5050",
  rvcModel:   "bandit",

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
