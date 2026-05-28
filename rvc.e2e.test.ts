/**
 * E2E tests for RVC container — run only when RVC server is reachable.
 *
 * Start server before running:
 *   podman start foni-rvc   (or: podman run -d --name foni-rvc -p 5050:5050 localhost/foni-rvc)
 *
 * Run:
 *   RVC_URL=http://127.0.0.1:5050 npm test -- rvc.e2e
 */

import { describe, it, expect, beforeAll } from "vitest";
import { RVCProcessor } from "./pipeline/processors.ts";

const RVC_URL = process.env.RVC_URL ?? "http://127.0.0.1:5050";

function makeWav(durationSecs = 0.1, sampleRate = 22050): Buffer {
  const numSamples = Math.floor(sampleRate * durationSecs);
  const dataSize = numSamples * 2;
  const buf = Buffer.alloc(44 + dataSize);
  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write("WAVE", 8);
  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28);
  buf.writeUInt16LE(2, 32);
  buf.writeUInt16LE(16, 34);
  buf.write("data", 36);
  buf.writeUInt32LE(dataSize, 40);
  return buf;
}

async function serverReachable(): Promise<boolean> {
  try {
    const r = await fetch(`${RVC_URL}/models`, { signal: AbortSignal.timeout(2000) });
    return r.ok;
  } catch {
    return false;
  }
}

describe("RVC container E2E", () => {
  let skip = false;

  beforeAll(async () => {
    skip = !(await serverReachable());
    if (skip) console.warn(`[e2e] RVC server not reachable at ${RVC_URL} — skipping`);
  });

  it("GET /models returns an array", async () => {
    if (skip) return;
    const r = await fetch(`${RVC_URL}/models`);
    expect(r.ok).toBe(true);
    const models = await r.json();
    expect(Array.isArray(models.models ?? models)).toBe(true);
  });

  it("GET /params returns expected fields", async () => {
    if (skip) return;
    const r = await fetch(`${RVC_URL}/params`);
    expect(r.ok).toBe(true);
    const params = await r.json();
    expect(params).toHaveProperty("f0method");
    expect(params).toHaveProperty("protect");
  });

  it("RVCProcessor falls back to input on server error", async () => {
    if (skip) return;
    const proc = new RVCProcessor(`${RVC_URL}/doesnotexist`);
    const input = Buffer.from("not-wav");
    const result = await proc.process(input);
    expect(result.equals(input)).toBe(true);
  }, 10_000);

  it("POST /convert with silence returns audio when model loaded", async () => {
    if (skip) return;

    const models = await fetch(`${RVC_URL}/models`).then(r => r.json());
    const list: string[] = models.models ?? models;
    if (list.length === 0) {
      console.warn("[e2e] No models in rvc_models/ — place a .pth in rvc/models/ and restart");
      return;
    }

    await fetch(`${RVC_URL}/models/${list[0]}`, { method: "POST" });
    const wav = makeWav(0.5); // 0.5s of silence
    console.time("[e2e] RVC CPU inference");
    const proc = new RVCProcessor(RVC_URL);
    const result = await proc.process(wav);
    console.timeEnd("[e2e] RVC CPU inference");
    expect(result).not.toBe(wav);
    expect(result.length).toBeGreaterThan(0);
  }, 120_000); // CPU inference can take up to 2 min
});
