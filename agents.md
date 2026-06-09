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

## Architecture — TTS pipeline

### RVC is deleted

RVC (voice conversion) was removed. The `rvc_model`, `rvc_url`, `rvc_enabled` fields in
`engine_config.rs` and `config.rs` are legacy dead weight — `_model` is `_`-prefixed and
ignored in `ModalSynthBackend::synthesize`. Do not reference RVC anywhere in new code.

### TTS backend: Chatterbox on Modal

All synthesis goes through `ModalSynthBackend` → `FONI_TTS_URL` (Chatterbox Multilingual on
Modal). Voice cloning is zero-shot: pass an `audio_prompt` (base64 WAV or file path) to clone
any speaker from ≥5 seconds of clean reference audio. No fine-tuning required.

Key Chatterbox parameters (passed through `SynthRequest` / `cloud_tts`):
- `language`      — BCP-47 code: `"en"`, `"ru"`, etc.
- `exaggeration`  — emotion intensity 0.3 (flat) → 1.5 (dramatic). Default 0.5.
- `cfg_weight`    — pace/guidance weight. Lower = freer prosody. Default 0.5. Use 0.0 for
                    cross-language voice transfer to reduce accent bleed.
- `temperature`   — prosody randomness 0.05 → 5.0. Default 0.8.
- `audio_prompt`  — **NOT YET WIRED.** Chatterbox accepts `audio_prompt_path` at inference
                    time. The fonictl `synth` command and the `/synthesize` route do not yet
                    pass a reference clip. This is the next gap to close.

### Voice persona capture pipeline

To capture a speaker's style and clone their voice:

```
fonictl fetch <youtube-url>          # download + convert to mono 24kHz WAV clips
fonictl tone  <clip.wav>             # arousal / dominance / valence → expression knobs
fonictl synth --voice en \
              --excitement  <X> \    # from tone output
              --assertiveness <X> \
              --warmth <X> \
              "text..."              # synthesize in that style
```

`tone` maps the acoustic profile of a reference clip to the three Chatterbox expression knobs.
Until `--audio-prompt` is wired, synthesis uses the Chatterbox default voice shaped by those
knobs — same energy and delivery style, different timbre.

### fonictl command map

| Command        | What it does |
|----------------|-------------|
| `fetch`        | Download YouTube/direct URL → mono 24kHz WAV, split by silence |
| `tone`         | Acoustic emotion profile → excitement/assertiveness/warmth knobs |
| `synth`        | Text → WAV via `/synthesize`. Supports `--voice en`, expression knobs |
| `analyse`      | Full acoustic metrics for a WAV (optionally vs a reference) |
| `process`      | Apply DSP chain to an existing WAV |
| `play`         | Play a WAV via system player |
| `clean`        | Trim silence, normalize volume, flag clipping across a dataset dir |
| `studio`       | Interactive maquette studio — produce N named variants, A/B compare |
| `mix`          | Interactive DSP mixer REPL |
| `listen`       | Render DSP stages or variants, play interactively |
| `render`       | Render a beat manifest (JSON) → single concatenated WAV |
| `bench`        | API round-trip latency benchmark |
| `tts-stats`    | Modal scaling status (backlog, runner count) |
| `tts-scale`    | Adjust Modal max containers / buffer |
| `compare`      | 1:1 studio vs synthetic test harness |
| `corpus`       | Acoustic fingerprint across a directory |
| `train`        | Full cloud training pipeline (clean → augment → train → compare → deploy) |

### MP4 production

There is no dedicated `fonictl mp4` command. Use ffmpeg after synthesis:

```bash
ffmpeg -f lavfi -i color=size=1280x720:rate=30:color=black \
       -i output.wav -c:v libx264 -c:a aac -shortest output.mp4
```

### Known gaps (next to fix)

1. `--audio-prompt <wav>` on `fonictl synth` — passes reference audio to Chatterbox for
   zero-shot voice cloning. Requires: adding the CLI flag, base64-encoding the WAV, adding
   `audio_prompt` to `SynthRequest`, and forwarding it in `cloud_tts()`.
2. `rvc_*` dead fields — should be removed from `FoniConfig` and `engine_config.rs`.
3. `ModalSynthBackend::synthesize` hardcodes `"language": "ru"` — should honour the
   `_model` parameter (repurposed as language/voice selector).
