import { describe, it, expect } from "vitest";
import {
  detectEmotion,
  updateEmotionState, effectiveWeights, neutralState, currentIntensity,
  NEUTRAL_WEIGHTS, EMOTION_PEAK_WEIGHTS,
  DEFAULT_HALF_LIFE_MS, FAST_DECAY_HALF_LIFE_MS,
} from "./core/emotion.ts";

// ─── detectEmotion ────────────────────────────────────────────────────────────

describe("detectEmotion", () => {
  describe("angry", () => {
    it("detects ALL CAPS words", () => {
      expect(detectEmotion("THIS IS BROKEN AGAIN").emotion).toBe("angry");
    });
    it("detects multiple exclamation marks", () => {
      expect(detectEmotion("Fix this now!!").emotion).toBe("angry");
    });
    it("detects aggressive English words", () => {
      expect(detectEmotion("this is fucking broken").emotion).toBe("angry");
    });
    it("detects aggressive Russian words", () => {
      expect(detectEmotion("какой идиот это написал").emotion).toBe("angry");
    });
  });

  describe("frustrated", () => {
    it("detects ellipsis", () => {
      expect(detectEmotion("it's broken again...").emotion).toBe("frustrated");
    });
    it("detects frustration words", () => {
      expect(detectEmotion("this ALWAYS happens, ugh").emotion).toBe("frustrated");
    });
    it("detects double question marks", () => {
      expect(detectEmotion("why is this still broken??").emotion).toBe("frustrated");
    });
    it("detects Russian frustration words", () => {
      expect(detectEmotion("опять та же ошибка, серьёзно").emotion).toBe("frustrated");
    });
  });

  describe("sarcastic", () => {
    it("detects English sarcasm markers", () => {
      expect(detectEmotion("oh great, it broke again").emotion).toBe("sarcastic");
    });
    it("detects Russian sarcasm markers", () => {
      expect(detectEmotion("ну конечно, само собой разумеется").emotion).toBe("sarcastic");
    });
    it("detects backhanded compliments", () => {
      expect(detectEmotion("thanks for nothing").emotion).toBe("sarcastic");
    });
  });

  describe("excited", () => {
    it("detects triple exclamation", () => {
      expect(detectEmotion("it works!!!").emotion).toBe("excited");
    });
    it("detects excited English words", () => {
      expect(detectEmotion("this is absolutely amazing and incredible").emotion).toBe("excited");
    });
    it("detects fire emoji", () => {
      expect(detectEmotion("yes!! 🔥🔥").emotion).toBe("excited");
    });
    it("detects Russian excited words", () => {
      expect(detectEmotion("это просто шикарно, невероятно!").emotion).toBe("excited");
    });
  });

  describe("cute", () => {
    it("detects over-polite English", () => {
      expect(detectEmotion("sorry to bother you, could you please help me?").emotion).toBe("cute");
    });
    it("detects Russian over-polite", () => {
      expect(detectEmotion("пожалуйста, не могли бы вы помочь?").emotion).toBe("cute");
    });
    it("detects uwu energy", () => {
      expect(detectEmotion("uwu could you help me pretty please").emotion).toBe("cute");
    });
    it("detects cute emoji", () => {
      expect(detectEmotion("please help me 🥺🙏").emotion).toBe("cute");
    });
  });

  describe("neutral", () => {
    it("returns neutral for plain statements", () => {
      expect(detectEmotion("deploy the service").emotion).toBe("neutral");
    });
    it("returns confidence=0 for neutral", () => {
      expect(detectEmotion("show me the logs").confidence).toBe(0);
    });
    it("returns empty signals for neutral", () => {
      expect(detectEmotion("update the config").signals).toHaveLength(0);
    });
  });

  describe("confidence", () => {
    it("confidence is between 0 and 1", () => {
      const inputs = ["THIS IS BROKEN!!!", "ugh...", "oh great", "amazing!!!", "please 🥺"];
      for (const input of inputs) {
        const { confidence } = detectEmotion(input);
        expect(confidence).toBeGreaterThanOrEqual(0);
        expect(confidence).toBeLessThanOrEqual(1);
      }
    });
    it("high signal count produces higher confidence", () => {
      const low  = detectEmotion("fix this");
      const high = detectEmotion("THIS IS FUCKING BROKEN!! idiot!!");
      expect(high.confidence).toBeGreaterThan(low.confidence);
    });
    it("populates signals list", () => {
      const { signals } = detectEmotion("THIS IS BROKEN!!");
      expect(signals.length).toBeGreaterThan(0);
    });
  });
});

// ─── Decay model ──────────────────────────────────────────────────────────────

describe("currentIntensity", () => {
  it("returns 0 for neutral state", () => {
    expect(currentIntensity(neutralState())).toBe(0);
  });

  it("returns initial intensity immediately after detection", () => {
    const state = { emotion: "angry" as const, intensity: 0.8, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
    expect(currentIntensity(state)).toBeCloseTo(0.8, 2);
  });

  it("halves intensity after one half-life", () => {
    const halfLifeMs = 1000;
    const state = { emotion: "angry" as const, intensity: 1.0, detectedAt: Date.now() - halfLifeMs, halfLifeMs };
    expect(currentIntensity(state)).toBeCloseTo(0.5, 2);
  });

  it("quarters intensity after two half-lives", () => {
    const halfLifeMs = 1000;
    const state = { emotion: "angry" as const, intensity: 1.0, detectedAt: Date.now() - 2 * halfLifeMs, halfLifeMs };
    expect(currentIntensity(state)).toBeCloseTo(0.25, 2);
  });

  it("returns 0 when decayed below INTENSITY_FLOOR", () => {
    const state = { emotion: "angry" as const, intensity: 0.05, detectedAt: Date.now() - 60_000, halfLifeMs: 1000 };
    expect(currentIntensity(state)).toBe(0);
  });
});

// ─── effectiveWeights ─────────────────────────────────────────────────────────

describe("effectiveWeights", () => {
  it("returns neutral weights when decayed to zero", () => {
    const state = { emotion: "angry" as const, intensity: 0.05, detectedAt: Date.now() - 60_000, halfLifeMs: 100 };
    const w = effectiveWeights(state);
    expect(w.matMultiplier).toBe(NEUTRAL_WEIGHTS.matMultiplier);
  });

  it("returns peak weights at full intensity", () => {
    const state = { emotion: "angry" as const, intensity: 1.0, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
    const w = effectiveWeights(state);
    expect(w.matMultiplier).toBeCloseTo(EMOTION_PEAK_WEIGHTS.angry.matMultiplier, 1);
  });

  it("interpolates matMultiplier at half intensity", () => {
    const state = { emotion: "angry" as const, intensity: 0.5, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
    const w = effectiveWeights(state);
    const expected = 1.0 + 0.5 * (EMOTION_PEAK_WEIGHTS.angry.matMultiplier - 1.0);
    expect(w.matMultiplier).toBeCloseTo(expected, 2);
  });

  it("snaps wordBias to neutral below 0.5 intensity", () => {
    const state = { emotion: "angry" as const, intensity: 0.4, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
    expect(effectiveWeights(state).wordBias).toBe("neutral");
  });

  it("uses peak wordBias at or above 0.5 intensity", () => {
    const state = { emotion: "angry" as const, intensity: 0.5, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
    expect(effectiveWeights(state).wordBias).toBe("aggressive");
  });

  it("excited reduces matMultiplier below 1", () => {
    const state = { emotion: "excited" as const, intensity: 1.0, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
    expect(effectiveWeights(state).matMultiplier).toBeLessThan(1);
  });
});

// ─── updateEmotionState ───────────────────────────────────────────────────────

describe("updateEmotionState", () => {
  it("reinforces same emotion — boosts intensity, resets clock", () => {
    const start = { emotion: "angry" as const, intensity: 0.5, detectedAt: Date.now() - 10_000, halfLifeMs: DEFAULT_HALF_LIFE_MS };
    const reading = detectEmotion("THIS IS BROKEN!!");
    const updated = updateEmotionState(start, reading);
    expect(updated.emotion).toBe("angry");
    expect(updated.intensity).toBeGreaterThan(currentIntensity(start));
    expect(updated.detectedAt).toBeGreaterThan(start.detectedAt);
  });

  it("shifts to stronger different emotion", () => {
    const start = { emotion: "cute" as const, intensity: 0.2, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
    const reading = detectEmotion("THIS IS FUCKING BROKEN!! YOU IDIOT");
    const updated = updateEmotionState(start, reading);
    expect(updated.emotion).toBe("angry");
  });

  it("does NOT shift when new emotion is weaker", () => {
    const start = { emotion: "angry" as const, intensity: 0.9, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
    const reading = detectEmotion("thanks 🥺"); // cute but weak
    const updated = updateEmotionState(start, reading);
    expect(updated.emotion).toBe("angry");
  });

  it("accelerates decay on neutral input", () => {
    const start = { emotion: "angry" as const, intensity: 0.8, detectedAt: Date.now(), halfLifeMs: DEFAULT_HALF_LIFE_MS };
    const reading = detectEmotion("update the config");
    const updated = updateEmotionState(start, reading);
    expect(updated.halfLifeMs).toBeLessThanOrEqual(FAST_DECAY_HALF_LIFE_MS);
  });

  it("restores full half-life on reinforcement", () => {
    const start = { emotion: "angry" as const, intensity: 0.5, detectedAt: Date.now(), halfLifeMs: FAST_DECAY_HALF_LIFE_MS };
    const reading = detectEmotion("THIS IS BROKEN!!");
    const updated = updateEmotionState(start, reading);
    expect(updated.halfLifeMs).toBe(DEFAULT_HALF_LIFE_MS);
  });

  it("caps intensity at 1.0 on repeated reinforcement", () => {
    let state = neutralState();
    const angry = "THIS IS FUCKING BROKEN!!! YOU IDIOT!!";
    for (let i = 0; i < 10; i++) {
      state = updateEmotionState(state, detectEmotion(angry));
    }
    expect(state.intensity).toBeLessThanOrEqual(1.0);
  });
});
