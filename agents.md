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
- WS test recv helpers must skip infrastructure messages via `support::is_infrastructure_msg`.
  Adding a new WS protocol message type? Add it to `tests/support/mod.rs::is_infrastructure_msg`
  if it is bookkeeping, not content.

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

## Architecture — TTS pipeline

### RVC is deleted

RVC (voice conversion) was removed. The `rvc_model`, `rvc_url`, `rvc_enabled` fields in
`engine_config.rs` and `config.rs` are legacy dead weight — `_model` is `_`-prefixed and
ignored in `ModalSynthBackend::synthesize`. Do not reference RVC anywhere in new code.

### TTS backend: Chatterbox on Modal

All synthesis goes through `ModalSynthBackend` → `DEPECHER_TTS_URL` (Chatterbox Multilingual on
Modal). Voice cloning is zero-shot: place a `reference.wav` + `lang` file under
`training/models/<model>/` to clone any speaker. The backend loads it automatically for both
the WS streaming path and the HTTP `/synthesize` route.

Key Chatterbox parameters (passed through `SynthRequest` / `cloud_tts`):
- `language`      — BCP-47 code: `"en"`, `"ru"`, etc. Resolved from `training/models/<model>/lang`.
- `exaggeration`  — emotion intensity 0.3 (flat) → 1.5 (dramatic). Default 0.5.
- `cfg_weight`    — pace/guidance weight. Lower = freer prosody. Default 0.5. Use 0.0 for
                    cross-language voice transfer to reduce accent bleed.
- `temperature`   — prosody randomness 0.05 → 5.0. Default 0.8.
- `audio_prompt`  — base64 WAV forwarded to Chatterbox for zero-shot voice cloning.
                    Loaded automatically from `training/models/<model>/reference.wav`.

### Voice model registration

To register a new voice model:

```
mkdir -p training/models/<name>/
ffmpeg -i <source.wav> -ss <start> -t <duration> -ar 24000 -ac 1 training/models/<name>/reference.wav
echo "en" > training/models/<name>/lang   # or "ru" etc.
depecherctl synth "test" --model <name> --no-dsp --out /tmp/test.wav
```

Best reference: 10–25 seconds of clean solo speech, 24kHz mono. No background noise,
no music, no other speakers. Use `depecherctl fetch` + manual curation to build the clip.

### Voice persona capture pipeline

To capture a speaker's style:

```
depecherctl fetch <youtube-url>          # download + convert to mono 24kHz WAV clips
depecherctl clean dataset/<name>/        # trim silence, normalize
depecherctl synth --model <name> "..."   # synthesize with zero-shot cloning
```

### depecherctl command map

| Command          | What it does |
|------------------|-------------|
| `fetch`          | Download YouTube/direct URL → mono 24kHz WAV, split by silence |
| `clean`          | Trim silence, normalize volume, flag clipping across a dataset dir |
| `augment`        | Speed perturbation to expand training data |
| `tone`           | Acoustic emotion profile → excitement/assertiveness/warmth knobs |
| `synth`          | Text → WAV via `/synthesize`. Supports `--model <name>`, expression knobs |
| `analyse`        | Full acoustic metrics for a WAV (optionally vs a reference) |
| `process`        | Apply DSP chain to an existing WAV |
| `play`           | Play a WAV via system player |
| `studio`         | Interactive maquette studio — produce N named variants, A/B compare |
| `mix`            | Interactive DSP mixer REPL |
| `listen`         | Render DSP stages or variants, play interactively |
| `render`         | Render a beat manifest (JSON) → single concatenated WAV |
| `probe`          | Single warmness ping to Modal TTS — reports ○ cold or ● warm + RTT |
| `dsp`            | Show live DSP config. `--reload` hot-reloads from `dsp-defaults.json` |
| `cache clear`    | Flush the server-side WAV LRU cache without restarting |
| `bench`          | API round-trip latency benchmark (sequential + parallel) |
| `tts-bench`      | Multi-round TTS latency benchmark with playback |
| `tts-stats`      | Modal scaling status (backlog, runner count) |
| `tts-scale`      | Adjust Modal max containers / buffer |
| `cost`           | Show Modal inference cost ledger |
| `compare`        | 1:1 studio vs synthetic test harness |
| `corpus`         | Acoustic fingerprint across a directory |
| `train`          | Full cloud training pipeline (clean → augment → train → compare → deploy) |
| `train-status`   | Check status of a Modal training job |
| `train-logs`     | Stream logs from a Modal training job |
| `train-cancel`   | Cancel a running Modal training job |
| `snapshot`       | Save current model scores as the baseline to beat |
| `compare-models` | Compare new model against saved baseline — auto pass/fail |

### Do not use bare shell for these operations

| Operation | Use instead |
|---|---|
| Check Modal backend warmness | `depecherctl probe` |
| Show/reload DSP config | `depecherctl dsp` / `depecherctl dsp --reload` |
| Flush WAV cache | `depecherctl cache clear` |
| Play a WAV | `depecherctl play <file>` |
| Kill audio playback | `pkill paplay` — add `depecherctl stop-audio` when needed twice |

### DSP config

Live defaults live in `training/dsp-defaults.json`. figment merge order:
`Rust defaults < YAML (foni-rvc.yaml) < JSON (dsp-defaults.json) < env vars`

The JSON file wins over Rust struct defaults. After editing, run `depecherctl dsp --reload`
to apply without restarting the server.

### MP4 production

No dedicated `depecherctl mp4` command. Use ffmpeg after synthesis:

```bash
ffmpeg -f lavfi -i color=size=1280x720:rate=30:color=black \
       -i output.wav -c:v libx264 -c:a aac -shortest output.mp4
```

### Known gaps

1. `depecherctl models` — list registered voice models with lang and reference status.
2. `depecherctl stop-audio` — kill active playback (currently `pkill paplay`).
3. `depecherctl modal logs/deploy/stop` — Modal app management without leaving depecherctl.
4. `rvc_*` dead fields — should be removed from `DepecherConfig` and `engine_config.rs`.
