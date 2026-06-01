/**
 * test/services.ts — service availability probes.
 *
 * Every probe returns a boolean and is safe to call in beforeAll().
 * Results are memoized so parallel test files don't hammer the same endpoint.
 */

import { Env } from "./env.ts";

// ─── Memoized probes ──────────────────────────────────────────────────────────

const cache = new Map<string, Promise<boolean>>();

function probe(key: string, fn: () => Promise<boolean>): Promise<boolean> {
  if (!cache.has(key)) cache.set(key, fn().catch(() => false));
  return cache.get(key)!;
}

// ─── Individual service probes ────────────────────────────────────────────────

export function synthAvailable(url = Env.SYNTH_URL ?? Env.RVC_URL): Promise<boolean> {
  return probe(`synth:${url}`, async () => {
    const r = await fetch(`${url}/params`, { signal: AbortSignal.timeout(2_000) });
    return r.ok;
  });
}

export function libreAvailable(url = "http://localhost:5000"): Promise<boolean> {
  return probe(`libre:${url}`, async () => {
    const r = await fetch(`${url}/languages`, { signal: AbortSignal.timeout(2_000) });
    return r.ok;
  });
}

export function ollamaAvailable(url = "http://localhost:11434"): Promise<boolean> {
  return probe(`ollama:${url}`, async () => {
    const r = await fetch(`${url}/api/tags`, { signal: AbortSignal.timeout(2_000) });
    return r.ok;
  });
}

export async function espeakAvailable(): Promise<boolean> {
  return probe("espeak", async () => {
    const { EspeakBackend } = await import("../backends/espeak.ts");
    return new EspeakBackend("ru").isAvailable();
  });
}

// ─── Composite status snapshot ────────────────────────────────────────────────

export interface ServiceStatus {
  espeak:   boolean;
  synth:    boolean;
  libre:    boolean;
  ollama:   boolean;
  synthUrl: string;
}

export async function checkServices(): Promise<ServiceStatus> {
  const synthUrl = Env.SYNTH_URL ?? Env.RVC_URL;
  const [espeak, synth, libre, ollama] = await Promise.all([
    espeakAvailable(),
    synthAvailable(synthUrl),
    libreAvailable(),
    ollamaAvailable(),
  ]);
  return { espeak, synth, libre, ollama, synthUrl };
}
