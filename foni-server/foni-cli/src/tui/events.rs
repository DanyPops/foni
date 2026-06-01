//! Keyboard event → state mutation. Returns true if the app should quit.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use foni_client::{SynthRequest, WavData, WireOpts};
use std::path::PathBuf;

use super::state::{InputMode, MixerApp, Panel, Track, PARAMS};

pub fn handle_key(app: &mut MixerApp, key: KeyEvent) -> bool {
    // Global: Ctrl-C always quits
    if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
        return true;
    }

    match &app.input_mode {
        InputMode::Normal => return handle_normal(app, key),
        InputMode::Rating { .. } => handle_rating(app, key),
        _ => handle_text_input(app, key),
    }
    false
}

// ── Normal mode ───────────────────────────────────────────────────────────────

fn handle_normal(app: &mut MixerApp, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') => return true,

        KeyCode::Tab | KeyCode::BackTab => {
            app.panel = app.panel.next();
        }
        KeyCode::F(1) => app.panel = Panel::Tracks,
        KeyCode::F(2) => app.panel = Panel::Params,
        KeyCode::F(3) => app.panel = Panel::Analyse,

        _ => match app.panel {
            Panel::Tracks => handle_tracks(app, key),
            Panel::Params => handle_params(app, key),
            Panel::Analyse => handle_analyse(app, key),
        },
    }
    false
}

fn handle_tracks(app: &mut MixerApp, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.selected + 1 < app.tracks.len() {
                app.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.selected = app.selected.saturating_sub(1);
        }

        // Play
        KeyCode::Enter => {
            if let Some(t) = app.tracks.get(app.selected) {
                if t.path.exists() {
                    let path = t.path.clone();
                    app.last_played = Some(app.selected);
                    spawn_play(path);
                } else {
                    app.status_msg = Some("not rendered yet — press n to render".into());
                }
            }
        }

        // Replay
        KeyCode::Char('r') => {
            if let Some(idx) = app.last_played {
                if let Some(t) = app.tracks.get(idx) {
                    if t.path.exists() {
                        spawn_play(t.path.clone());
                    }
                }
            }
        }

        // Rate 1-5 — enter rating sub-mode
        KeyCode::Char(c @ '1'..='5') => {
            let score = c as u8 - b'0';
            app.input_mode = InputMode::Rating {
                track_idx: app.selected,
                score,
            };
            app.status_msg = Some(format!(
                "Rating {} ★{}/5 — Enter confirm | n for note | Esc cancel",
                app.tracks
                    .get(app.selected)
                    .map(|t| t.label.as_str())
                    .unwrap_or("?"),
                score,
            ));
        }

        // Winner
        KeyCode::Char('w') => {
            app.set_winner(app.selected);
            app.save_session();
        }

        // Fork to Params scratchpad
        KeyCode::Char('f') => {
            app.fork_from(app.selected);
            app.panel = Panel::Params;
        }

        // Render new track from scratchpad
        KeyCode::Char('n') => {
            app.input_mode = InputMode::RenderName { buf: String::new() };
        }

        // Drop track
        KeyCode::Char('d') if !app.tracks.is_empty() => {
            let label = app.tracks[app.selected].label.clone();
            app.tracks.remove(app.selected);
            if app.selected >= app.tracks.len() && app.selected > 0 {
                app.selected -= 1;
            }
            app.status_msg = Some(format!("dropped {label}"));
            app.save_session();
        }

        // Note
        KeyCode::Char('e') => {
            let existing = app
                .tracks
                .get(app.selected)
                .and_then(|t| t.note.clone())
                .unwrap_or_default();
            app.input_mode = InputMode::Note {
                track_idx: app.selected,
                buf: existing,
            };
        }

        _ => {}
    }
}

fn handle_params(app: &mut MixerApp, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.param_sel + 1 < PARAMS.len() {
                app.param_sel += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.param_sel = app.param_sel.saturating_sub(1);
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if let Some(p) = PARAMS.get(app.param_sel) {
                let cur = app.scratch_get(p.key);
                let next = (cur - p.step).clamp(p.min, p.max);
                // Round to step precision to avoid float drift
                let next = (next / p.step).round() * p.step;
                app.scratch_set(p.key, next);
            }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if let Some(p) = PARAMS.get(app.param_sel) {
                let cur = app.scratch_get(p.key);
                let next = (cur + p.step).clamp(p.min, p.max);
                let next = (next / p.step).round() * p.step;
                app.scratch_set(p.key, next);
            }
        }
        // Render scratchpad as new track
        KeyCode::Enter => {
            app.input_mode = InputMode::RenderName { buf: String::new() };
        }
        // Fork from currently selected track
        KeyCode::Char('f') => {
            let idx = app.selected;
            app.fork_from(idx);
        }
        _ => {}
    }
}

fn handle_analyse(app: &mut MixerApp, key: KeyEvent) {
    if key.code == KeyCode::Enter {
        // Signal to run_tui to trigger an analyse call
        app.analysing = true;
    }
}

// ── Rating sub-mode ───────────────────────────────────────────────────────────

fn handle_rating(app: &mut MixerApp, key: KeyEvent) {
    let (track_idx, score) = match app.input_mode {
        InputMode::Rating { track_idx, score } => (track_idx, score),
        _ => return,
    };

    match key.code {
        KeyCode::Enter => {
            app.set_rating(track_idx, score, None);
            app.save_session();
            app.input_mode = InputMode::Normal;
            app.status_msg = Some(format!(
                "rated {} ★{}/5",
                app.tracks
                    .get(track_idx)
                    .map(|t| t.label.as_str())
                    .unwrap_or("?"),
                score,
            ));
        }
        KeyCode::Char('n') => {
            // Switch to note entry for this rating
            app.set_rating(track_idx, score, None);
            app.input_mode = InputMode::Note {
                track_idx,
                buf: String::new(),
            };
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.status_msg = None;
        }
        KeyCode::Char(c @ '1'..='5') => {
            let new_score = c as u8 - b'0';
            app.input_mode = InputMode::Rating {
                track_idx,
                score: new_score,
            };
            app.status_msg = Some(format!(
                "Rating {} ★{}/5 — Enter confirm | n note | Esc cancel",
                app.tracks
                    .get(track_idx)
                    .map(|t| t.label.as_str())
                    .unwrap_or("?"),
                new_score,
            ));
        }
        _ => {}
    }
}

// ── Text input ────────────────────────────────────────────────────────────────

fn handle_text_input(app: &mut MixerApp, key: KeyEvent) {
    match &app.input_mode {
        InputMode::Note { track_idx, buf } => {
            let (idx, mut buf) = (*track_idx, buf.clone());
            match key.code {
                KeyCode::Enter => {
                    app.tracks.get_mut(idx).map(|t| t.note = Some(buf.clone()));
                    app.save_session();
                    app.input_mode = InputMode::Normal;
                    app.status_msg = Some(format!("note saved"));
                }
                KeyCode::Esc => {
                    app.input_mode = InputMode::Normal;
                }
                KeyCode::Backspace => {
                    buf.pop();
                    app.input_mode = InputMode::Note {
                        track_idx: idx,
                        buf,
                    };
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    app.input_mode = InputMode::Note {
                        track_idx: idx,
                        buf,
                    };
                }
                _ => {}
            }
        }
        InputMode::RenderName { buf } => {
            let mut buf = buf.clone();
            match key.code {
                KeyCode::Enter if !buf.is_empty() => {
                    app.rendering = true;
                    app.status_msg = Some(format!("rendering «{buf}»…"));
                    // Signal stored in input_mode for run_tui to pick up
                    app.input_mode = InputMode::RenderName { buf };
                }
                KeyCode::Esc => {
                    app.input_mode = InputMode::Normal;
                }
                KeyCode::Backspace => {
                    buf.pop();
                    app.input_mode = InputMode::RenderName { buf };
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    app.input_mode = InputMode::RenderName { buf };
                }
                _ => {}
            }
        }
        _ => {}
    }
}

// ── Audio playback ────────────────────────────────────────────────────────────

/// Spawn paplay in a detached thread — does not block the TUI render loop.
pub fn spawn_play(path: PathBuf) {
    std::thread::spawn(move || {
        for player in &["paplay", "aplay", "afplay", "mpv", "ffplay"] {
            if std::process::Command::new(player)
                .arg(&path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return;
            }
        }
    });
}

// ── Async actions driven from run_tui ─────────────────────────────────────────

/// Call synthesize (no DSP) then process with scratch opts.
/// Returns the rendered WAV bytes.
pub async fn do_render(
    client: &foni_client::FoniClient,
    phrase: &str,
    model: &str,
    opts: WireOpts,
) -> Result<WavData, foni_client::FoniError> {
    let req = SynthRequest {
        text: phrase.to_string(),
        model: Some(model.to_string()),
        dsp: false,
        prosody: false,
        ..SynthRequest::new(phrase)
    };
    let base = client.synthesize(&req).await?;
    client.process(&base, opts).await
}

/// Run acoustic analysis vs Sidorovich reference.
pub async fn do_analyse(
    client: &foni_client::FoniClient,
    wav: &WavData,
    ref_path: Option<&std::path::Path>,
) -> Result<foni_client::AnalyseResponse, foni_client::FoniError> {
    let reference = if let Some(p) = ref_path {
        std::fs::read(p).ok().map(WavData)
    } else {
        None
    };
    client
        .analyse(wav, reference.as_ref(), Some("trader1a"))
        .await
}
