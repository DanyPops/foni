//! Ratatui TUI mixer — entry point.
pub mod events;
pub mod state;
pub mod ui;

use std::{path::PathBuf, time::Duration};

use crossterm::event::{self, Event, KeyEventKind};
use depecher_client::{DepecherClient, WireOpts};
use ratatui::DefaultTerminal;

use state::{load_session, session_path, InputMode, MixerApp, Track};
use tracing::info;

/// Boot the TUI mixer.  Returns when the user presses q / Ctrl-C.
pub fn run(
    rt: &tokio::runtime::Runtime,
    server: &str,
    phrase: &str,
    model: &str,
    initial_tracks: Vec<Track>,
    reference: Option<std::path::PathBuf>,
) {
    let client = DepecherClient::new(server);
    let mut app = MixerApp::new(
        client.clone(),
        phrase.to_string(),
        model.to_string(),
        initial_tracks,
    );
    app.reference = reference;

    // Restore ratings from previous session if phrase matches.
    if let Some(prev) = load_session() {
        if prev.phrase == phrase {
            for t in app.tracks.iter_mut() {
                if let Some(r) = prev.tracks.iter().find(|r| r.label == t.label) {
                    t.rating = r.rating;
                    t.note = r.note.clone();
                    t.winner = r.winner.unwrap_or(false);
                }
            }
            app.status_msg = Some(format!("session restored ({} tracks)", prev.tracks.len()));
        }
    }

    let mut terminal = ratatui::init();
    let result = run_loop(rt, &mut terminal, &mut app, &client);
    ratatui::restore();

    app.save_session();
    println!("\n  session saved → {}", session_path().display());

    if let Err(e) = result {
        info!("TUI error: {e}");
    }
}

fn run_loop(
    rt: &tokio::runtime::Runtime,
    terminal: &mut DefaultTerminal,
    app: &mut MixerApp,
    client: &DepecherClient,
) -> std::io::Result<()> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        // Clear one-shot messages after they've been shown.
        if !matches!(app.input_mode, InputMode::Rating { .. }) {
            app.status_msg = None;
        }

        // ── Async action: render new track ────────────────────────────────────
        if app.rendering {
            let label = match &app.input_mode {
                InputMode::RenderName { buf } => buf.clone(),
                _ => String::new(),
            };
            if !label.is_empty() {
                let opts = app.scratch_as_wire_opts();
                let path = std::env::temp_dir()
                    .join(format!("depecherctl_mix_{}.wav", label.replace(' ', "_")));
                let desc = format_opts_desc(&app.scratch);
                let phrase = app.phrase.clone();
                let model = app.model.clone();
                let c = client.clone();

                match rt.block_on(events::do_render(&c, &phrase, &model, opts.clone())) {
                    Ok(wav) => {
                        std::fs::write(&path, wav.as_bytes()).ok();
                        // Replace or append.
                        if let Some(t) = app.tracks.iter_mut().find(|t| t.label == label) {
                            t.path = path;
                            t.opts = opts;
                            t.desc = desc;
                        } else {
                            app.tracks.push(Track {
                                label: label.clone(),
                                desc,
                                path,
                                opts,
                                rating: None,
                                note: None,
                                winner: false,
                                analyse: None,
                            });
                            app.selected = app.tracks.len() - 1;
                        }
                        app.status_msg = Some(format!("✓  rendered «{label}»"));
                        app.save_session();
                    }
                    Err(e) => {
                        app.status_msg = Some(format!("❌  render failed: {e}"));
                    }
                }
            }
            app.rendering = false;
            app.input_mode = InputMode::Normal;
        }

        // ── Async action: analyse selected track ──────────────────────────────
        if app.analysing {
            if let Some(t) = app.tracks.get(app.selected).cloned() {
                if t.path.exists() {
                    let wav = WireOpts::default(); // placeholder — we read file bytes
                    let _ = wav; // suppress unused
                    let bytes = std::fs::read(&t.path).unwrap_or_default();
                    let wav_data = depecher_client::WavData(bytes);
                    let ref_path = PathBuf::from("baseline/stalker/wav/sidorovich/trader1a.wav");
                    let ref_opt = if ref_path.exists() {
                        Some(ref_path.as_path())
                    } else {
                        None
                    };

                    match rt.block_on(events::do_analyse(client, &wav_data, ref_opt)) {
                        Ok(a) => {
                            let rms = a.analysis.loudness.rms_db;
                            app.status_msg =
                                Some(format!("analysed «{}»  RMS {rms:.1} dBFS", t.label));
                            if let Some(tr) = app.tracks.get_mut(app.selected) {
                                tr.analyse = Some(a);
                            }
                        }
                        Err(e) => {
                            app.status_msg = Some(format!("❌  analyse failed: {e}"));
                        }
                    }
                } else {
                    app.status_msg = Some("track not rendered yet".into());
                }
            }
            app.analysing = false;
        }

        // ── Input event ───────────────────────────────────────────────────────
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && events::handle_key(app, key) {
                    break;
                }
            }
        }

        if app.quit {
            break;
        }
    }
    Ok(())
}

fn format_opts_desc(opts: &serde_json::Value) -> String {
    let get = |k: &str| opts.get(k).and_then(|v| v.as_f64());
    let mut parts = Vec::new();
    if let Some(v) = get("vibratoDepth") {
        if v > 0.0 {
            parts.push(format!("vib={v:.3}"));
        }
    }
    if let Some(v) = get("compressionRatio") {
        parts.push(format!("comp={v:.1}"));
    }
    if let Some(v) = get("tiltLowDb") {
        parts.push(format!("lo={v:+.0}"));
    }
    if let Some(v) = get("tiltHighDb") {
        parts.push(format!("hi={v:+.0}"));
    }
    if let Some(v) = get("rmsTargetLufs") {
        parts.push(format!("rms={v:.0}"));
    }
    if parts.is_empty() {
        "defaults".into()
    } else {
        parts.join(" ")
    }
}
