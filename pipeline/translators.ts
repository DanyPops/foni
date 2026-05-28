import type { Translator } from "./interfaces.ts";

export class IdentityTranslator implements Translator {
  async translate(text: string): Promise<string> {
    return text;
  }
}

export class MyMemoryTranslator implements Translator {
  constructor(private readonly from: string, private readonly to: string) {}

  async translate(text: string): Promise<string> {
    try {
      const url = `https://api.mymemory.translated.net/get?q=${encodeURIComponent(text)}&langpair=${this.from}|${this.to}`;
      const resp = await fetch(url, { signal: AbortSignal.timeout(5_000) });
      if (!resp.ok) return text;
      const data = await resp.json() as { responseData: { translatedText: string } };
      return data.responseData.translatedText || text;
    } catch {
      return text;
    }
  }
}
