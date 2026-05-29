#!/usr/bin/env node
/**
 * Tuning listener — derive config names from tuning.e2e.test.ts, run vitest.
 *
 * Single source of truth: CONFIGS array in tuning.e2e.test.ts.
 * No duplication in package.json — names are read at runtime.
 *
 * Usage:
 *   node scripts/listen.mjs        → play all configs
 *   node scripts/listen.mjs 3      → play config 3
 *   node scripts/listen.mjs list   → show available configs
 */

import { readFileSync } from "node:fs";
import { spawnSync }    from "node:child_process";
import { resolve }      from "node:path";

const root    = resolve(import.meta.dirname, "..");
const src     = readFileSync(resolve(root, "tuning.e2e.test.ts"), "utf8");

// Extract slugs from:  { slug: "...", opts: ... }
// Then build names the same way CONFIGS does: "${i+1}. ${slug}"
const slugs = [...src.matchAll(/\{\s*slug:\s*"([^"]+)"/g)].map(m => m[1]);
const names = slugs.map((slug, i) => `${i + 1}. ${slug}`);

if (names.length === 0) {
  console.error("No configs found in tuning.e2e.test.ts");
  process.exit(1);
}

const arg = process.argv[2];

// list
if (arg === "list") {
  names.forEach(n => console.info(`  ${n}`));
  process.exit(0);
}

// single config by number
const filter = arg ? names[parseInt(arg) - 1] : undefined;

if (arg && !filter) {
  console.error(`Config ${arg} not found. Run: node scripts/listen.mjs list`);
  process.exit(1);
}

const vitestArgs = ["vitest", "run", "tuning.e2e"];
if (filter) vitestArgs.push("-t", filter);

const result = spawnSync("npx", vitestArgs, {
  stdio: "inherit",
  cwd:   root,
  env:   process.env,
});

process.exit(result.status ?? 0);
