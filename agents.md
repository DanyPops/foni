# Foni — Agent Rules

## Scribe

- **Always use the `scribe_artifact` MCP tool** to create/update tasks and docs — never manipulate `~/.scribe/scribe.sqlite` or `~/.local/share/scribe/scribe.sqlite` directly with Python/sqlite3.
- File tasks BEFORE implementing. Use Locus to analyse the impact surface first.
- Scope is `foni`. Campaign refs: `FON-CMP-1` (Model Forge), `FON-CMP-2` (Rust engine).

## Testing

- **Never run ad-hoc `node -e` or `python3 -c` one-liners to debug logic.** Write a test instead.
- All tests live in `*.test.ts` files at the repo root alongside the code they cover.
- Test runner: `npx vitest run` — must stay green before every commit.
- Snapshot tests: strip ANSI, use `toMatchSnapshot()`. Behaviour tests: assert action calls.
- No magic values in tests — use the same named constants as production code.

## Git

- Commit after every coherent unit of work (file + tests + green).
- Push to `danypops` remote after every commit: `git push danypops master`.
- Commit message format: `<type>: <description> (<task-refs>)`.

## Code structure

- `core/` — zero pi/ExtensionAPI imports. All domain logic lives here.
- `index.ts` — thin pi adapter only. No domain logic, no config defaults, no pipeline construction.
- `tui/` — pi-specific UI components only.
- Dependency rule: `extension → core`, never `core → extension`.

## Named constants

- No magic numbers or strings. Every threshold, weight, and default gets a named export.
- For-humans rule: code is read more than written — name things for the next reader.
