/**
 * Emotion Heuristic Detector + Decay Model
 *
 * Detects emotional register from user input text and produces
 * dynamic probability multipliers for the mat/interject pipeline.
 *
 * Decay model: exponential half-life, lazily evaluated at pipeline build time.
 *   intensity(t) = initial × exp(−ln(2) × elapsed / halfLifeMs)
 *   effectiveWeight = neutral + intensity(t) × (peak − neutral)
 *
 * No timers. No external state. Pure math at call time.
 */

// ─── Types ────────────────────────────────────────────────────────────────────

export type Emotion =
  | "angry"       // mirror aggression back — max mat, heat 3 only
  | "frustrated"  // commiserate — elevated mat, lower energy
  | "sarcastic"   // be more sarcastic back — ironic interjections
  | "excited"     // match energy — lots of exclamations, less mat
  | "cute"        // mock them for being a sissy — mockery injections
  | "neutral";    // default weights

// WordBias lives in pipeline/interfaces.ts to avoid pipeline→core circular import.
// Imported here for use within this file; re-exported for backward compat.
import type { WordBias } from "../pipeline/interfaces.ts";
export type { WordBias };

export interface EmotionWeights {
  /** Multiplier on config.matProb. 2.0 = double. */
  matMultiplier:       number;
  /** Multiplier on config.interjectProb. */
  interjectMultiplier: number;
  /** Minimum heat level from lexicon. */
  heatMin:             1 | 2 | 3;
  /** Which word sub-list to prefer. */
  wordBias:            WordBias;
}

export interface EmotionReading {
  emotion:    Emotion;
  /** 0–1 confidence from signal strength. */
  confidence: number;
  /** Human-readable triggered heuristics (for debugging / status bar). */
  signals:    string[];
}

// ─── Peak weights per emotion ─────────────────────────────────────────────────

export const EMOTION_PEAK_WEIGHTS: Record<Emotion, EmotionWeights> = {
  angry:      { matMultiplier: 2.0, interjectMultiplier: 0.5, heatMin: 3, wordBias: "aggressive"    },
  frustrated: { matMultiplier: 1.5, interjectMultiplier: 1.0, heatMin: 2, wordBias: "commiseration" },
  sarcastic:  { matMultiplier: 0.8, interjectMultiplier: 2.0, heatMin: 1, wordBias: "mockery"       },
  excited:    { matMultiplier: 0.3, interjectMultiplier: 3.0, heatMin: 1, wordBias: "excitement"    },
  cute:       { matMultiplier: 1.5, interjectMultiplier: 2.5, heatMin: 1, wordBias: "mockery"       },
  neutral:    { matMultiplier: 1.0, interjectMultiplier: 1.0, heatMin: 1, wordBias: "neutral"       },
};

export const NEUTRAL_WEIGHTS = EMOTION_PEAK_WEIGHTS.neutral;

// ─── Status bar emoji ─────────────────────────────────────────────────────────

export const EMOTION_EMOJI: Record<Emotion, string> = {
  angry:      "😤",
  frustrated: "😒",
  sarcastic:  "🙄",
  excited:    "🤩",
  cute:       "🫵",  // pointing — "you, sissy"
  neutral:    "",
};

// ─── Bias-specific word overrides ─────────────────────────────────────────────
//
// When wordBias is set, the injection pipeline substitutes these lists
// in place of the default MAT/INTERJECT arrays.

export const BIAS_WORDS: import("../pipeline/interfaces.ts").BiasWordMap = {
  aggressive: {
    suffix:     [", блядь", ", сука", ", пиздец", ", ёб твою мать"],
    standalone: ["блядь", "сука", "пиздец", "охуеть"],
    prefix:     ["Ёбаный в рот,", "Какого хуя,", "Мать твою,"],
  },
  commiseration: {
    suffix:     [", пиздец", ", капец", ", беспредел", ", бля"],
    standalone: ["капец", "кирдык", "пиздец", "ёпта"],
    prefix:     ["Ну и дела,", "Беспредел полный,", "Чёрт возьми,"],
  },
  mockery: {
    suffix:     [", ха!", ", нежная душа!", ", цаца!", ", слюнтяй!"],
    standalone: ["Ха!", "Нежная душа!", "Цаца какая!", "Слюнтяй!", "Размазня!", "Ой, бедняжка!"],
    prefix:     ["Ой, ну надо же,", "Смотри-ка,", "Вот те раз,"],
  },
  excitement: {
    suffix:     [", нихуя себе!", ", вот это да!", ", нехило!", ", ого!"],
    standalone: ["Нихуя себе!", "Ого!", "Вот это да!", "Охуеть!", "Нехило!"],
    prefix:     ["Ого,", "Ну ничего себе,", "Вот это поворот,"],
  },
  neutral: {
    suffix:     [", блядь", ", сука", ", ёпта", ", бля"],
    standalone: ["блядь", "сука", "ёпта", "пиздец"],
    prefix:     ["Ёбаный в рот,", "Мать твою,", "Какого хуя,"],
  },
};

// ─── Heuristic detector ───────────────────────────────────────────────────────

type ScoreMap = Record<Exclude<Emotion, "neutral">, number>;

/**
 * Detect emotional register from user message text.
 * Pure function — no state, no side effects.
 */
export function detectEmotion(text: string): EmotionReading {
  const signals: string[] = [];
  const scores: ScoreMap = { angry: 0, frustrated: 0, sarcastic: 0, excited: 0, cute: 0 };

  // ── ANGRY signals ──────────────────────────────────────────────────────────
  const capsWords = text.split(/\s+/).filter(w => /^[A-ZА-ЯЁ]{3,}$/.test(w));
  if (capsWords.length >= 2 || (capsWords.length === 1 && text.trim().split(/\s+/).length <= 3)) {
    scores.angry += 2; signals.push("caps-lock");
  }
  if (/[!]{2,}/.test(text) && !/[!]{3,}/.test(text)) {
    scores.angry += 1; signals.push("double-exclamation");
  }
  if (/\b(wtf|fuck|shit|damn|idiot|stupid|useless|broken|crap|hell)\b/i.test(text)) {
    scores.angry += 2; signals.push("aggressive-en-words");
  }
  if (/(идиот|тупой|сломал|бесполезн|дурак|чёрт|чёрт возьми)/i.test(text)) {
    scores.angry += 2; signals.push("aggressive-ru-words");
  }
  if (text.trim().split(/\s+/).length < 6 && /[!?]/.test(text) && !/[!]{3,}/.test(text)) {
    scores.angry += 1; signals.push("short-aggressive");
  }

  // ── FRUSTRATED signals ─────────────────────────────────────────────────────
  if (/\.{3,}/.test(text)) {
    scores.frustrated += 2; signals.push("ellipsis");
  }
  if (/\b(ugh|argh|sigh|again|still|always|never|every time|seriously|how many)\b/i.test(text)) {
    scores.frustrated += 2; signals.push("frustration-en-words");
  }
  if (/(опять|снова|всегда|никогда|серьёзно|сколько раз|ну сколько)/i.test(text)) {
    scores.frustrated += 2; signals.push("frustration-ru-words");
  }
  if (/[?]{2,}/.test(text)) {
    scores.frustrated += 1; signals.push("double-question");
  }
  if (/(why|почему|зачем).{0,30}[?!]/i.test(text)) {
    scores.frustrated += 1; signals.push("why-question");
  }

  // ── SARCASTIC signals ──────────────────────────────────────────────────────
  if (/\b(oh great|oh sure|just perfect|wonderful|brilliant|just what i needed|love it|fantastic)\b/i.test(text)) {
    scores.sarcastic += 3; signals.push("sarcasm-en-markers");
  }
  if (/(ну конечно|само собой|ещё бы|надо же|вот уж|неудивительно)/i.test(text)) {
    scores.sarcastic += 3; signals.push("sarcasm-ru-markers");
  }
  if (/\b(obviously|clearly|of course|totally|definitely|naturally)\b/i.test(text)) {
    scores.sarcastic += 1; signals.push("irony-adverbs");
  }
  if (/\b(thanks for nothing|great job|well done|nice work)\b/i.test(text)) {
    scores.sarcastic += 2; signals.push("backhanded-compliment");
  }

  // ── EXCITED signals ────────────────────────────────────────────────────────
  if (/[!]{3,}/.test(text)) {
    scores.excited += 2; signals.push("triple-exclamation");
  }
  if (/\b(amazing|awesome|incredible|genius|perfect|exactly|love this|this is it)\b/i.test(text)) {
    scores.excited += 2; signals.push("excited-en-words");
  }
  if (/(отлично|шикарно|гениально|невероятно|обалдеть|класс|круто)/i.test(text)) {
    scores.excited += 2; signals.push("excited-ru-words");
  }
  if (/[🔥💯🎉✨🚀😍🤯⚡]/u.test(text)) {
    scores.excited += 3; signals.push("excitement-emoji");
  }
  if (/\b(omg|omfg|holy shit|no way|wow)\b/i.test(text)) {
    scores.excited += 1; signals.push("excited-en-interjections");
  }

  // ── CUTE signals ───────────────────────────────────────────────────────────
  if (/\b(please|kindly|would you mind|could you possibly|pretty please|if you don't mind|sorry to bother)\b/i.test(text)) {
    scores.cute += 3; signals.push("over-polite-en");
  }
  if (/(пожалуйста|не могли бы вы|будьте добры|извините что беспокою|спасибо большое)/i.test(text)) {
    scores.cute += 3; signals.push("over-polite-ru");
  }
  if (/\b(uwu|owo|hehe|teehee|heehee)\b/i.test(text)) {
    scores.cute += 4; signals.push("uwu-energy");
  }
  if (/[💕💖💗💓💞❤️🥺👉👈😊🙏🫶]/u.test(text)) {
    scores.cute += 2; signals.push("cute-emoji");
  }
  if (/\b(you're amazing|you're the best|you're so smart|so helpful)\b/i.test(text)) {
    scores.cute += 2; signals.push("excessive-flattery");
  }
  if (/~/.test(text)) {
    scores.cute += 1; signals.push("tilde-suffix");
  }

  // ── Find winner ────────────────────────────────────────────────────────────
  const entries = Object.entries(scores) as Array<[Exclude<Emotion, "neutral">, number]>;
  const [topEmotion, topScore] = entries.reduce((best, curr) => curr[1] > best[1] ? curr : best);

  if (topScore === 0) {
    return { emotion: "neutral", confidence: 0, signals: [] };
  }

  // Confidence: scale by score, cap at 1. Score of 4+ = high confidence.
  const confidence = Math.min(1, topScore / 4);

  return { emotion: topEmotion, confidence, signals };
}

// ─── Emotion State + Decay ────────────────────────────────────────────────────

/** Default half-life: 5 minutes. After 5 min idle, intensity halves. */
export const DEFAULT_HALF_LIFE_MS = 5 * 60 * 1000;

/** Accelerated half-life when shifting away or user sends neutral message. */
export const FAST_DECAY_HALF_LIFE_MS = 60 * 1000;

/** Intensity floor below which we treat as neutral. */
export const INTENSITY_FLOOR = 0.1;

export interface EmotionState {
  emotion:    Emotion;
  intensity:  number;   // 0–1
  detectedAt: number;   // Date.now()
  halfLifeMs: number;
}

export function neutralState(): EmotionState {
  return { emotion: "neutral", intensity: 0, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
}

/**
 * Compute decayed intensity at the current moment.
 * Uses exponential decay: I(t) = I₀ × 2^(−elapsed/halfLife)
 */
export function currentIntensity(state: EmotionState, now = Date.now()): number {
  if (state.emotion === "neutral") return 0;
  const elapsed = now - state.detectedAt;
  const decayed = state.intensity * Math.pow(2, -elapsed / state.halfLifeMs);
  return decayed < INTENSITY_FLOOR ? 0 : decayed;
}

/**
 * Interpolate between neutral and peak weights using current decayed intensity.
 * At intensity=1.0: peak weights. At intensity=0: neutral weights.
 */
export function effectiveWeights(state: EmotionState, now = Date.now()): EmotionWeights {
  const intensity = currentIntensity(state, now);
  if (intensity === 0) return NEUTRAL_WEIGHTS;

  const peak = EMOTION_PEAK_WEIGHTS[state.emotion];

  return {
    matMultiplier:       lerp(NEUTRAL_WEIGHTS.matMultiplier,       peak.matMultiplier,       intensity),
    interjectMultiplier: lerp(NEUTRAL_WEIGHTS.interjectMultiplier, peak.interjectMultiplier, intensity),
    // heatMin and wordBias snap at half intensity to avoid blurry middle state
    heatMin:    intensity >= 0.5 ? peak.heatMin    : NEUTRAL_WEIGHTS.heatMin,
    wordBias:   intensity >= 0.5 ? peak.wordBias   : NEUTRAL_WEIGHTS.wordBias,
  };
}

function lerp(a: number, b: number, t: number): number {
  return a + t * (b - a);
}

/**
 * Process a new user message and return the updated EmotionState.
 * Pure function — returns new state, does not mutate.
 */
export function updateEmotionState(current: EmotionState, reading: EmotionReading, now = Date.now()): EmotionState {
  if (reading.emotion === "neutral" || reading.confidence === 0) {
    // Neutral input — accelerate decay but don't force reset
    return { ...current, halfLifeMs: Math.min(current.halfLifeMs, FAST_DECAY_HALF_LIFE_MS) };
  }

  const curIntensity = currentIntensity(current, now);

  if (reading.emotion === current.emotion) {
    // Reinforce — boost intensity, reset clock, restore half-life
    return {
      emotion:    current.emotion,
      intensity:  Math.min(1, curIntensity + reading.confidence * 0.35),
      detectedAt: now,
      halfLifeMs: DEFAULT_HALF_LIFE_MS,
    };
  }

  // Different emotion
  if (reading.confidence >= curIntensity) {
    // New emotion is stronger — full shift
    return {
      emotion:    reading.emotion,
      intensity:  reading.confidence,
      detectedAt: now,
      halfLifeMs: DEFAULT_HALF_LIFE_MS,
    };
  }

  // New emotion is weaker — partial blend, accelerate decay of current
  return { ...current, intensity: curIntensity * 0.6, halfLifeMs: FAST_DECAY_HALF_LIFE_MS };
}
