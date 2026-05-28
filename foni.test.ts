
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  stripMarkdown,
  drainChunks,
  freshState,
  resolveBacktickRun,
  buildFakeYouHeaders,
  FAKEYOU_CDN,
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

  it("splits on sentence boundary (period + space + capital)", () => {
    const { chunks, remainder } = drainChunks(
      "Hello there. World is great."
    );
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

  it("handles multiple sentences", () => {
    const { chunks } = drainChunks(
      "One sentence. Two sentence. Three sentence. Incomplete"
    );
    expect(chunks.length).toBeGreaterThanOrEqual(2);
    expect(chunks[0]).toBe("One sentence.");
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

describe("buildFakeYouHeaders", () => {
  it("includes content-type and accept always", () => {
    const h = buildFakeYouHeaders("");
    expect(h["Content-Type"]).toBe("application/json");
    expect(h["Accept"]).toBe("application/json");
  });

  it("adds authorization when api key provided", () => {
    const h = buildFakeYouHeaders("my-key");
    expect(h["Authorization"]).toBe("Bearer my-key");
  });

  it("omits authorization when api key is empty", () => {
    const h = buildFakeYouHeaders("");
    expect(h["Authorization"]).toBeUndefined();
  });
});

describe("FAKEYOU_CDN", () => {
  it("points to google storage", () => {
    expect(FAKEYOU_CDN).toContain("googleapis.com");
  });
});

// ─── synthesizeFakeYou (fetch-mocked) ────────────────────────────────────────

describe("synthesizeFakeYou via fetch mock", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("throws when inference endpoint returns non-ok", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValueOnce({
        ok: false,
        status: 429,
        text: async () => "rate limited",
      })
    );

    const { FakeYouBackend } = await import("./backends/fakeyou.ts");
    const b = new FakeYouBackend("weight_test", "", 0);
    await expect(b.synthesize("hello", { voice: "en_0", speed: 1 })).rejects.toThrow("429");
  });

  it("polls until complete_success and downloads audio", async () => {
    const audioBytes = new Uint8Array([82, 73, 70, 70]); // WAV RIFF header
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValueOnce({
          ok: true,
          json: async () => ({ inference_job_token: "JTINF:abc123" }),
        })
        .mockResolvedValueOnce({
          ok: true,
          json: async () => ({
            state: {
              status: "complete_success",
              maybe_public_bucket_wav_audio_path: "/test/audio.wav",
            },
          }),
        })
        .mockResolvedValueOnce({
          ok: true,
          arrayBuffer: async () => audioBytes.buffer,
        })
    );

    const { FakeYouBackend } = await import("./backends/fakeyou.ts");
    const b = new FakeYouBackend("weight_test", "", 0);
    await expect(b.synthesize("hello", { voice: "en_0", speed: 1 })).resolves.not.toThrow();
  });
});

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
