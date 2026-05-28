# Foni 🔊

Pi TTS extension — streams pi's assistant output through a local Text-to-Speech engine with optional RVC voice conversion.

## Features

- **Multiple backends**: Silero · Kokoro · FakeYou · espeak-ng · say (macOS)
- **RVC voice conversion**: speak as Sidorovich, STALKER bandits, Warcraft heroes, or any HuggingFace RVC model
- **Russian output**: auto-translate responses via MyMemory before speaking
- **Interactive TUI**: live status widget, `↑↓` model picker overlay
- **ROGYB-tested**: 32 tests covering pure functions, fetch-mocked backends, E2E RVC container

## Install

```bash
pi install git:github.com/danypops/foni
```

## Quick start

```
/tts          ← toggle on (auto-detects espeak-ng, Silero, Kokoro)
/tts test     ← diagnostic — shows ✓/✗ for every step
/tts status   ← current config
```

## RVC bandit voice (Cheeki Breeki)

```bash
# Start the RVC container (auto-loads bandit model)
systemctl --user start foni-rvc   # after first-time setup below
```

First-time setup:
```bash
cd ~/.config/pi/agent/git/github.com/danypops/foni

# Download Grigoriy German's bandit voice (Cheeki Breeki)
mkdir -p rvc/models/bandit
curl -L https://huggingface.co/Warlock700/German-Bandits/resolve/main/german-bandit-fp32-comp-de_esser-no_noise-norm.zip \
  -o /tmp/bandit.zip
unzip /tmp/bandit.zip -d rvc/models/bandit
mv rvc/models/bandit/german-bandit-fp32-comp-de_esser-no_noise-norm/* rvc/models/bandit/
rmdir rvc/models/bandit/german-bandit-fp32-comp-de_esser-no_noise-norm

# Build and enable the RVC container
cd rvc && podman build -t foni-rvc .
cp foni-rvc.service ~/.config/systemd/user/
systemctl --user daemon-reload && systemctl --user enable --now foni-rvc
```

Then in pi:
```
/tts rvc on
/tts lang ru    ← optional: speak Russian (auto-translated)
/tts
```

## RVC config (`rvc/foni-rvc.yaml`)

```yaml
model: bandit          # auto-loaded on container start
params:
  f0up_key: -2         # semitones (negative = lower/gruffer)
  index_rate: 0.77     # 0–1: character voice strength
  protect: 0.33        # 0–1: accent bleed-through
```

Edit and `systemctl --user restart foni-rvc` — no container rebuild needed.

## Other STALKER voices

Browse the [Warlock700 STALKER collection](https://huggingface.co/collections/Warlock700/rvc-stalker-voices) (35+ voices) and the [Warcraft III collection](https://huggingface.co/collections/Warlock700/rvc-warcraft-iii-voices) (20+ voices).

```bash
mkdir -p rvc/models/sidorovich
curl -L https://huggingface.co/bobpingvin/Sidorovich/resolve/main/sidorovich.zip \
  -o /tmp/sidorovich.zip && unzip /tmp/sidorovich.zip -d rvc/models/sidorovich
```

Then `/tts rvc model` (no args) → interactive picker.

## Commands

| Command | Description |
|---------|-------------|
| `/tts` | Toggle on/off |
| `/tts test` | Step-by-step diagnostic |
| `/tts status` | Current config |
| `/tts voice <name>` | Switch voice (Silero: en_0–en_117) |
| `/tts speed <n>` | 0.5–3.0 |
| `/tts lang en\|ru` | Language (ru = MyMemory translation) |
| `/tts backend silero\|kokoro\|fakeyou\|espeak\|auto` | Force backend |
| `/tts rvc on\|off` | Enable/disable RVC conversion |
| `/tts rvc model [name]` | Load model (no arg = interactive picker) |
| `/tts rvc models` | List models on server |
| `/tts stop` | Kill queued audio |
| `/tts search <query>` | Search FakeYou TTS voices |
| `/tts token weight_xxx` | Set FakeYou model token |
