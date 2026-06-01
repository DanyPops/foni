
import { describe, it, expect, vi, beforeEach } from "vitest";
import { AudioLRU, SpeakFacade } from "./pipeline/speak-facade.ts";
import { makeITGlossaryMiddleware, IT_GLOSSARY, compose } from "./pipeline/translators.ts";
import type { TextCtx } from "./pipeline/translators.ts";
import {
  stripMarkdown,
  drainChunks,
  freshState,
  resolveBacktickRun,
  buildFakeYouHeaders,
} from "./lib.ts";

// ─── stripMarkdown ────────────────────────────────────────────────────────────

describe("stripMarkdown", () => {
  it("strips fenced code blocks", () => {
    const input = "Hello\n```ts\nconst x = 1;\n```\nworld";
    expect(stripMarkdown(input)).toBe("Hello\nworld");
  });

  it("strips inline code", () => {
    expect(stripMarkdown("use `npm install` to install")).toBe(
      "use  to install"
    );
  });

  it("strips headers", () => {
    expect(stripMarkdown("## Hello\nworld")).toBe("Hello\nworld");
  });

  it("preserves link text, drops URL", () => {
    expect(stripMarkdown("[click here](https://example.com)")).toBe(
      "click here"
    );
  });

  it("strips bold and italic", () => {
    expect(stripMarkdown("**bold** and _italic_")).toBe("bold and italic");
  });

  it("strips blockquote markers", () => {
    expect(stripMarkdown("> some quote")).toBe("some quote");
  });

  it("strips unordered list bullets", () => {
    expect(stripMarkdown("- item one\n- item two")).toBe(
      "item one\nitem two"
    );
  });

  it("strips ordered list numbers", () => {
    expect(stripMarkdown("1. first\n2. second")).toBe("first\nsecond");
  });

  it("collapses multiple blank lines", () => {
    expect(stripMarkdown("a\n\n\n\nb")).toBe("a\n\nb");
  });

  it("returns clean prose unchanged", () => {
    expect(stripMarkdown("Hello world.")).toBe("Hello world.");
  });
});

// ─── drainChunks ─────────────────────────────────────────────────────────────

describe("drainChunks", () => {
  it("returns empty chunks and full remainder for mid-sentence text", () => {
    const { chunks, remainder } = drainChunks("Hello there");
    expect(chunks).toHaveLength(0);
    expect(remainder).toBe("Hello there");
  });

  it("splits on sentence boundary (period + space)", () => {
    const { chunks, remainder } = drainChunks("Hello there. World is great.");
    expect(chunks).toContain("Hello there.");
    expect(remainder).toContain("World");
  });

  it("splits on paragraph break", () => {
    const { chunks, remainder } = drainChunks("First para.\n\nSecond para.");
    expect(chunks).toContain("First para.");
    expect(remainder).toContain("Second para");
  });

  it("skips chunks shorter than 3 chars", () => {
    const { chunks } = drainChunks("Hi.\n\nHello world.");
    expect(chunks.every((c) => c.length > 2)).toBe(true);
  });

  it("handles multiple English sentences", () => {
    const { chunks } = drainChunks(
      "One sentence. Two sentence. Three sentence. Incomplete"
    );
    expect(chunks.length).toBeGreaterThanOrEqual(2);
    expect(chunks[0]).toBe("One sentence.");
  });

  it("splits Russian sentences (Cyrillic capitals — previously broken)", () => {
    const { chunks, remainder } = drainChunks(`Да. Нет. Хорошо. Незаконченный`);
    expect(chunks).toContain("Да.");
    expect(chunks).toContain("Нет.");
    expect(chunks).toContain("Хорошо.");
    expect(remainder).toBe("Незаконченный");
  });

  it("each Russian sentence becomes a separate cache key", () => {
    const text = "Нужно сделать. Понял. Готово.";
    const { chunks } = drainChunks(text + " ");
    // Previously returned 1 chunk (whole paragraph)
    // Now returns 3 separate cacheable sentences
    expect(chunks).toHaveLength(3);
    expect(chunks[0]).toBe("Нужно сделать.");
    expect(chunks[1]).toBe("Понял.");
    expect(chunks[2]).toBe("Готово.");
  });

  it("splits on ! and ? too", () => {
    const { chunks } = drainChunks("Ого! Ну и ну? Хорошо. Дальше");
    expect(chunks).toContain("Ого!");
    expect(chunks).toContain("Ну и ну?");
    expect(chunks).toContain("Хорошо.");
  });
});

// ─── StreamState ──────────────────────────────────────────────────────────────

describe("freshState", () => {
  it("returns zeroed state", () => {
    const s = freshState();
    expect(s.buffer).toBe("");
    expect(s.codeDepth).toBe(0);
    expect(s.inInlineCode).toBe(false);
    expect(s.backtickRun).toBe(0);
  });
});

describe("resolveBacktickRun", () => {
  it("triple backtick opens a code fence", () => {
    const s = freshState();
    s.backtickRun = 3;
    resolveBacktickRun(s);
    expect(s.codeDepth).toBe(1);
    expect(s.backtickRun).toBe(0);
  });

  it("triple backtick closes an open code fence", () => {
    const s = freshState();
    s.codeDepth = 1;
    s.backtickRun = 3;
    resolveBacktickRun(s);
    expect(s.codeDepth).toBe(0);
  });

  it("single backtick toggles inline code", () => {
    const s = freshState();
    s.backtickRun = 1;
    resolveBacktickRun(s);
    expect(s.inInlineCode).toBe(true);
    s.backtickRun = 1;
    resolveBacktickRun(s);
    expect(s.inInlineCode).toBe(false);
  });

  it("zero run is a no-op", () => {
    const s = freshState();
    resolveBacktickRun(s);
    expect(s.codeDepth).toBe(0);
    expect(s.inInlineCode).toBe(false);
  });
});

// ─── FakeYou helpers ──────────────────────────────────────────────────────────



// ─── synthesizeFakeYou (fetch-mocked) ────────────────────────────────────────


// ─── convertWithRVC (fetch-mocked) ───────────────────────────────────────────

describe("convertWithRVC via fetch mock", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("falls back to input on fetch error (non-fatal)", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockRejectedValueOnce(new Error("ECONNREFUSED"))
    );
    const { RVCProcessor } = await import("./pipeline/processors.ts");
    const proc = new RVCProcessor("http://127.0.0.1:5050");
    const input = Buffer.from("test");
    const result = await proc.process(input);
    expect(result.equals(input)).toBe(true);
  });

  it("falls back to input on non-ok response", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValueOnce({ ok: false, status: 500 })
    );
    const { RVCProcessor } = await import("./pipeline/processors.ts");
    const proc = new RVCProcessor("http://127.0.0.1:5050");
    const input = Buffer.from("test");
    const result = await proc.process(input);
    expect(result.equals(input)).toBe(true);
  });
});


// ─── AudioLRU ─────────────────────────────────────────────────────────────────

describe("AudioLRU", () => {
  it("returns undefined on miss", () => {
    const lru = new AudioLRU(1024);
    expect(lru.get("nope")).toBeUndefined();
  });

  it("returns stored buffer on hit", () => {
    const lru = new AudioLRU(1024);
    const buf = Buffer.from("audio");
    lru.set("k", buf);
    expect(lru.get("k")).toBe(buf);
  });

  it("tracks byte size correctly", () => {
    const lru = new AudioLRU(1024);
    lru.set("a", Buffer.alloc(100));
    lru.set("b", Buffer.alloc(200));
    expect(lru.bytes).toBe(300);
    expect(lru.size).toBe(2);
  });

  it("evicts LRU entry when over budget", () => {
    const lru = new AudioLRU(300);
    lru.set("a", Buffer.alloc(100));
    lru.set("b", Buffer.alloc(100));
    lru.set("c", Buffer.alloc(100));
    expect(lru.size).toBe(3);
    lru.set("d", Buffer.alloc(100));      // evicts "a" (LRU)
    expect(lru.get("a")).toBeUndefined();
    expect(lru.get("b")).toBeDefined();
    expect(lru.size).toBe(3);
    expect(lru.bytes).toBeLessThanOrEqual(300);
  });

  it("get() promotes entry — protected from next eviction", () => {
    const lru = new AudioLRU(200);
    lru.set("a", Buffer.alloc(100));
    lru.set("b", Buffer.alloc(100));
    lru.get("a");                         // promote "a" to MRU
    lru.set("c", Buffer.alloc(100));      // evicts LRU — now "b"
    expect(lru.get("a")).toBeDefined();
    expect(lru.get("b")).toBeUndefined();
  });

  it("overwriting a key updates size correctly", () => {
    const lru = new AudioLRU(1024);
    lru.set("k", Buffer.alloc(100));
    lru.set("k", Buffer.alloc(50));
    expect(lru.bytes).toBe(50);
    expect(lru.size).toBe(1);
  });

  it("clear() resets everything", () => {
    const lru = new AudioLRU(1024);
    lru.set("a", Buffer.alloc(100));
    lru.clear();
    expect(lru.size).toBe(0);
    expect(lru.bytes).toBe(0);
    expect(lru.get("a")).toBeUndefined();
  });
});

// ─── SpeakFacade audio cache ──────────────────────────────────────────────────

describe("SpeakFacade audio cache", () => {
  function makeFacade(cache?: AudioLRU) {
    const translator = { translate: vi.fn(async (t: string) => t) };
    const backend    = {
      name:        "mock",
      synthesize:  vi.fn(async () => Buffer.from("audio-bytes")),
      isAvailable: vi.fn(async () => true),
    };
    const processor  = { process: vi.fn(async (b: Buffer) => b) };
    const playerPlays: Buffer[] = [];
    const player     = {
      detected: () => "mock" as const,
      play:     vi.fn(async (b: Buffer) => { playerPlays.push(b); }),
    };
    const facade = new SpeakFacade(
      translator as any, backend as any, processor as any, player as any,
      { voice: "ru", speed: 1.0 },
      cache,
    );
    return { facade, translator, backend, processor, player, playerPlays };
  }

  it("first call synthesises and stores in cache", async () => {
    const { facade, backend } = makeFacade();
    await facade.speak("Привет");
    expect(backend.synthesize).toHaveBeenCalledOnce();
    expect(facade.cache.size).toBe(1);
  });

  it("second identical call is a cache hit — backend not called again", async () => {
    const { facade, backend, playerPlays } = makeFacade();
    await facade.speak("Привет");
    await facade.speak("Привет");
    expect(backend.synthesize).toHaveBeenCalledOnce();
    expect(playerPlays).toHaveLength(2);
  });

  it("different text produces a cache miss", async () => {
    const { facade, backend } = makeFacade();
    await facade.speak("Привет");
    await facade.speak("Пока");
    expect(backend.synthesize).toHaveBeenCalledTimes(2);
  });

  it("cache clear causes re-synthesis on next call", async () => {
    const { facade, backend } = makeFacade();
    await facade.speak("Привет");
    facade.cache.clear();
    await facade.speak("Привет");
    expect(backend.synthesize).toHaveBeenCalledTimes(2);
  });

  it("shared cache across two facades avoids double synthesis", async () => {
    const shared = new AudioLRU();
    const { facade: f1 } = makeFacade(shared);
    const { facade: f2, backend: b2 } = makeFacade(shared);
    await f1.speak("Привет");
    await f2.speak("Привет");
    expect(b2.synthesize).not.toHaveBeenCalled();
  });

  it("cacheStats() reflects current state", async () => {
    const { facade } = makeFacade();
    await facade.speak("Привет");
    const stats = facade.cacheStats();
    expect(stats).toContain("1 entries");
    expect(stats).toContain("MB");
  });
});

// ─── SpeakFacade queue + cancellation ────────────────────────────────────────

describe("SpeakFacade serial play queue", () => {
  function makeFacade() {
    const playOrder: string[] = [];
    const synthOrder: string[] = [];

    const translator = { translate: vi.fn(async (t: string) => t) };
    const backend = {
      name: "mock",
      isAvailable: vi.fn(async () => true),
      synthesize: vi.fn(async (text: string) => {
        synthOrder.push(text);
        return Buffer.from(text);
      }),
    };
    const processor = { process: vi.fn(async (b: Buffer) => b) };
    const player = {
      detected: () => "mock" as const,
      play: vi.fn(async (buf: Buffer) => {
        playOrder.push(buf.toString());
      }),
    };
    const facade = new SpeakFacade(
      translator as any, backend as any, processor as any, player as any,
      { voice: "ru", speed: 1.0 },
    );
    return { facade, playOrder, synthOrder, backend, player };
  }

  it("two concurrent speaks play in call order", async () => {
    const { facade, playOrder } = makeFacade();
    // Fire both without awaiting the first
    const p1 = facade.speak("first");
    const p2 = facade.speak("second");
    await Promise.all([p1, p2]);
    expect(playOrder).toEqual(["first", "second"]);
  });

  it("three concurrent speaks play in call order", async () => {
    const { facade, playOrder } = makeFacade();
    await Promise.all([
      facade.speak("one"),
      facade.speak("two"),
      facade.speak("three"),
    ]);
    expect(playOrder).toEqual(["one", "two", "three"]);
  });

  it("stop() cancels pending speaks — they do not play", async () => {
    const { facade, player } = makeFacade();
    // Enqueue three items then stop immediately
    const p1 = facade.speak("one");
    const p2 = facade.speak("two");
    const p3 = facade.speak("three");
    facade.stop();                        // cancel generation
    await Promise.all([p1, p2, p3]);
    // At most the first could have already entered synthesis before stop()
    expect(player.play).not.toHaveBeenCalledWith(Buffer.from("two"));
    expect(player.play).not.toHaveBeenCalledWith(Buffer.from("three"));
  });

  it("speak() after stop() works normally", async () => {
    const { facade, playOrder } = makeFacade();
    await facade.speak("before");
    facade.stop();
    await facade.speak("after");
    expect(playOrder).toContain("after");
  });

  it("play queue recovers after synthesis error", async () => {
    const { facade, player, backend } = makeFacade();
    backend.synthesize
      .mockRejectedValueOnce(new Error("network"))  // first call fails
      .mockResolvedValue(Buffer.from("ok"));         // subsequent calls work
    await facade.speak("bad");
    await facade.speak("good");
    // First speak errored — not played. Second should still play.
    expect(player.play).toHaveBeenCalledOnce();
  });

  it("Promise.all() prewarm: all phrases synthesised, played in order", async () => {
    const { facade, backend, playOrder } = makeFacade();
    const phrases = ["Да.", "Нет.", "Хорошо.", "Понял."];

    await Promise.all(phrases.map(p => facade.speak(p)));

    // Every phrase was synthesised exactly once
    expect(backend.synthesize).toHaveBeenCalledTimes(phrases.length);
    // Played in call order
    expect(playOrder).toEqual(phrases);
  });

  it("speak() resolves only after playback completes", async () => {
    const events: string[] = [];
    const { facade, player } = makeFacade();
    player.play.mockImplementation(async (buf: Buffer) => {
      events.push(`start:${buf.toString()}`);
      await new Promise(r => setTimeout(r, 10)); // simulate playback time
      events.push(`end:${buf.toString()}`);
    });
    await facade.speak("hello");
    events.push("resolved");
    expect(events).toEqual(["start:hello", "end:hello", "resolved"]);
  });
});

// ─── IT Glossary middleware ─────────────────────────────────────────────────────

describe("makeITGlossaryMiddleware", () => {
  function runGlossary(text: string): string {
    const mw  = makeITGlossaryMiddleware();
    const ctx: TextCtx = { input: text, text, lang: "en" };
    // run synchronously via the compose helper
    let result = text;
    mw(ctx, async () => {}).then(() => { result = ctx.text; });
    // Force microtask — glossary is sync inside async wrapper
    return ctx.text;
  }

  it("replaces 'deploy' with 'деплой'", () => {
    expect(runGlossary("Deploy the service.")).toContain("деплой");
  });

  it("replaces 'commit' with 'коммит'", () => {
    expect(runGlossary("Commit the changes.")).toContain("коммит");
  });

  it("replaces 'merge' with 'мерж'", () => {
    expect(runGlossary("Merge the branch.")).toContain("мерж");
  });

  it("replaces 'pull request' with 'пуллреквест'", () => {
    expect(runGlossary("Open a pull request.")).toContain("пуллреквест");
  });

  it("keeps API, SQL, CI/CD unchanged", () => {
    const result = runGlossary("Call the API and run SQL via CI/CD.");
    expect(result).toContain("API");
    expect(result).toContain("SQL");
    expect(result).toContain("CI/CD");
  });

  it("handles multiple replacements in one sentence", () => {
    const result = runGlossary("Deploy the service, commit changes, and merge the branch.");
    expect(result).toContain("деплой");
    expect(result).toContain("коммит");
    expect(result).toContain("мерж");
  });

  it("is case-insensitive", () => {
    expect(runGlossary("DEPLOY the service.")).toContain("деплой");
    expect(runGlossary("Debug the issue.")).toContain("дебаг");
  });

  it("completes in under 5ms (zero latency)", () => {
    const start = Date.now();
    for (let i = 0; i < 100; i++) {
      runGlossary("Deploy the service, commit changes, merge the branch, debug the issue.");
    }
    expect(Date.now() - start).toBeLessThan(50); // 100 runs in <50ms
  });
});
