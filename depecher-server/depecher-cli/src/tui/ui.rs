//! Ratatui render functions — pure, no side effects.
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Row, Table, TableState},
    Frame,
};

use super::state::{InputMode, MixerApp, Panel, PARAMS};

// ── Colour palette ────────────────────────────────────────────────────────────

const C_ACCENT: Color = Color::Cyan;
const C_WINNER: Color = Color::Yellow;
const C_DIM: Color = Color::DarkGray;
const C_OK: Color = Color::Green;
const C_WARN: Color = Color::Yellow;
const C_FAR: Color = Color::Red;
const C_SELECTED: Color = Color::Blue;

// ── Top-level render ──────────────────────────────────────────────────────────

pub fn render(f: &mut Frame, app: &MixerApp) {
    let area = f.area();

    // Outer layout: header | body | footer
    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(0),    // body
        Constraint::Length(1), // footer
    ])
    .split(area);

    render_header(f, chunks[0], app);
    render_body(f, chunks[1], app);
    render_footer(f, chunks[2], app);

    // Overlay input prompt if active
    if matches!(
        app.input_mode,
        InputMode::Note { .. } | InputMode::RenderName { .. }
    ) {
        render_input_overlay(f, area, app);
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &MixerApp) {
    let tabs: Vec<Span> = [Panel::Tracks, Panel::Params, Panel::Analyse]
        .iter()
        .map(|&p| {
            let label = format!(" {} ", p.label());
            if p == app.panel {
                Span::styled(
                    label,
                    Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(label, Style::default().fg(C_DIM))
            }
        })
        .collect();

    let phrase_preview: String = app.phrase.chars().take(48).collect();
    let line = Line::from(vec![
        Span::styled(" 🎛 Foni Mixer", Style::default().fg(C_ACCENT).bold()),
        Span::raw("  "),
        Span::styled(format!("«{phrase_preview}»"), Style::default().fg(C_DIM)),
        Span::raw("   "),
    ])
    .patch_style(Style::default());

    let tab_line = Line::from(tabs);

    let header = Paragraph::new(vec![line, tab_line]);
    f.render_widget(header, area);
}

fn render_body(f: &mut Frame, area: Rect, app: &MixerApp) {
    match app.panel {
        Panel::Tracks => render_tracks(f, area, app),
        Panel::Params => render_params(f, area, app),
        Panel::Analyse => render_analyse(f, area, app),
    }
}

fn render_footer(f: &mut Frame, area: Rect, app: &MixerApp) {
    let msg = if let Some(ref s) = app.status_msg {
        s.as_str()
    } else {
        match app.panel {
            Panel::Tracks  => "j/k select  Enter play  1-5 rate  w winner  f fork→Params  n render  d drop  Tab panel  q quit",
            Panel::Params  => "j/k param  ←/→ adjust  Enter render-new  f fork from selected  Tab panel",
            Panel::Analyse => "Enter analyse selected track  Tab panel",
        }
    };
    let line = Line::from(Span::styled(format!(" {msg}"), Style::default().fg(C_DIM)));
    f.render_widget(Paragraph::new(line), area);
}

// ── Tracks panel ──────────────────────────────────────────────────────────────

fn render_tracks(f: &mut Frame, area: Rect, app: &MixerApp) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Tracks ")
        .title_style(Style::default().fg(C_ACCENT));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.tracks.is_empty() {
        f.render_widget(
            Paragraph::new("  No tracks. Run depecherctl mix --diagnose first.")
                .style(Style::default().fg(C_DIM)),
            inner,
        );
        return;
    }

    // Each track = 3 lines: info | bar | blank separator
    let row_height = 3u16;
    let visible = (inner.height / row_height) as usize;
    let start = app.selected.saturating_sub(visible.saturating_sub(1));

    for (local_i, track_i) in (start..).take(visible).enumerate() {
        let Some(t) = app.tracks.get(track_i) else {
            break;
        };
        let y = inner.y + (local_i as u16 * row_height);
        if y + 2 >= inner.y + inner.height {
            break;
        }

        let selected = track_i == app.selected;
        let style = if selected {
            Style::default().bg(C_SELECTED).fg(Color::White)
        } else {
            Style::default()
        };

        // Row 1: ✪ N  label            ★★★★☆  note
        let crown = if t.winner { "✪ " } else { "  " };
        let dot = if t.path.exists() { "●" } else { "◦" };
        let stars = t.stars();
        let note_snippet: String = t.note.as_deref().unwrap_or("").chars().take(24).collect();
        let row1 = format!(
            " {dot}{crown}{:>2}  {:<20}  {stars}  {note_snippet}",
            track_i + 1,
            &t.label.chars().take(20).collect::<String>(),
        );
        let row1_style = if t.winner { style.fg(C_WINNER) } else { style };
        f.render_widget(
            Paragraph::new(row1).style(row1_style),
            Rect::new(inner.x, y, inner.width, 1),
        );

        // Row 2: VU bar  |  rms_db
        let rms = t.rms_db().unwrap_or(-40.0);
        let rms_clamped = ((rms + 40.0) / 40.0).clamp(0.0, 1.0); // -40..0 dBFS → 0..1
        let bar_w = inner.width.saturating_sub(20);
        let bar_area = Rect::new(inner.x + 4, y + 1, bar_w, 1);
        let label_area = Rect::new(inner.x + 4 + bar_w, y + 1, 14, 1);

        let bar_color = if rms > -10.0 { C_WARN } else { Color::Green };
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(bar_color).bg(Color::DarkGray))
            .ratio(rms_clamped as f64)
            .label("");
        f.render_widget(gauge, bar_area);

        let rms_label = if t.analyse.is_some() {
            format!(" {rms:+.1} dBFS")
        } else if t.path.exists() {
            " ─ (not analysed)".into()
        } else {
            " ─ (not rendered)".into()
        };
        f.render_widget(
            Paragraph::new(rms_label).style(Style::default().fg(C_DIM)),
            label_area,
        );
    }
}

// ── Params panel ──────────────────────────────────────────────────────────────

fn render_params(f: &mut Frame, area: Rect, app: &MixerApp) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" Params — Scratchpad ")
        .title_style(Style::default().fg(C_ACCENT));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    for (i, p) in PARAMS.iter().enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.y + inner.height {
            break;
        }

        let val = app.scratch_get(p.key);
        let ratio = ((val - p.min) / (p.max - p.min)).clamp(0.0, 1.0);
        let selected = i == app.param_sel && app.panel == Panel::Params;

        let bar_color = if selected { C_ACCENT } else { Color::Green };
        let label_style = if selected {
            Style::default().fg(C_ACCENT).bold()
        } else {
            Style::default()
        };

        // Label col
        let label_w = 18u16;
        let val_w = 12u16;
        let bar_w = inner.width.saturating_sub(label_w + val_w + 4);

        let arrow = if selected { "▶ " } else { "  " };
        f.render_widget(
            Paragraph::new(format!("{arrow}{:<16}", p.label)).style(label_style),
            Rect::new(inner.x, y, label_w, 1),
        );

        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(bar_color).bg(Color::DarkGray))
            .ratio(ratio as f64)
            .label("");
        f.render_widget(gauge, Rect::new(inner.x + label_w, y, bar_w, 1));

        f.render_widget(
            Paragraph::new(format!("  {}", (p.format)(val))).style(Style::default().fg(C_DIM)),
            Rect::new(inner.x + label_w + bar_w, y, val_w, 1),
        );
    }
}

// ── Analyse panel ─────────────────────────────────────────────────────────────

fn render_analyse(f: &mut Frame, area: Rect, app: &MixerApp) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" Analyse ")
        .title_style(Style::default().fg(C_ACCENT));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    if app.analysing {
        f.render_widget(
            Paragraph::new(" ⟳  Loading acoustic metrics…").style(Style::default().fg(C_DIM)),
            inner,
        );
        return;
    }

    let Some(track) = app.selected_track() else {
        f.render_widget(Paragraph::new(" No track selected."), inner);
        return;
    };

    let Some(ref a) = track.analyse else {
        f.render_widget(
            Paragraph::new(format!(
                " Track «{}» — press Enter to analyse vs Sidorovich reference.",
                track.label
            ))
            .style(Style::default().fg(C_DIM)),
            inner,
        );
        return;
    };

    // Summary line
    let summary = format!(
        " {} — RMS {:.1} dBFS  |  F0 {:.0} Hz  |  Voiced {:.0}%  |  Dur {:.2}s",
        track.label,
        a.analysis.loudness.rms_db,
        a.analysis.pitch.pitch_hz,
        a.analysis.pitch.voice_presence * 100.0,
        a.analysis.temporal.duration_secs,
    );
    f.render_widget(
        Paragraph::new(summary).style(Style::default().fg(Color::White).bold()),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    // Gap table
    if let Some(ref gap) = a.gap_result {
        let header = Row::new(["Metric", "Target", "Actual", "Gap%", "Verdict"])
            .style(Style::default().fg(C_ACCENT).bold());

        let rows: Vec<Row> = gap
            .rows
            .iter()
            .map(|r| {
                let color = match r.verdict.as_str() {
                    s if s.contains("close") => C_OK,
                    s if s.contains("near") => C_WARN,
                    s if s.contains("far") => C_FAR,
                    _ => C_FAR,
                };
                Row::new(vec![
                    r.metric.clone(),
                    format!("{:.2}", r.target),
                    format!("{:.2}", r.actual),
                    format!("{:.1}%", r.gap_pct),
                    r.verdict.clone(),
                ])
                .style(Style::default().fg(color))
            })
            .collect();

        let mean_row = Row::new(vec![
            "── Mean gap".into(),
            String::new(),
            String::new(),
            format!("{:.1}%", gap.mean_gap_pct),
            String::new(),
        ])
        .style(Style::default().fg(C_DIM).bold());

        let mut all_rows = rows;
        all_rows.push(mean_row);

        let table = Table::new(
            all_rows,
            [
                Constraint::Fill(2),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(7),
                Constraint::Fill(1),
            ],
        )
        .header(header)
        .column_spacing(1);

        let table_area = Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(2),
        );
        let mut ts = TableState::default();
        f.render_stateful_widget(table, table_area, &mut ts);
    }
}

// ── Input overlay ─────────────────────────────────────────────────────────────

fn render_input_overlay(f: &mut Frame, area: Rect, app: &MixerApp) {
    let (title, buf) = match &app.input_mode {
        InputMode::Note { track_idx, buf } => {
            let label = app
                .tracks
                .get(*track_idx)
                .map(|t| t.label.as_str())
                .unwrap_or("?");
            (
                format!(" Note for «{label}» (Enter confirm, Esc cancel) "),
                buf.as_str(),
            )
        }
        InputMode::RenderName { buf } => (
            " New track name (Enter render, Esc cancel) ".into(),
            buf.as_str(),
        ),
        _ => return,
    };

    let w = (area.width * 3 / 4).max(40);
    let x = (area.width - w) / 2;
    let y = area.height / 2;
    let popup = Rect::new(x, y, w, 3);

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(format!(" {buf}▌"))
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(Style::default().bg(Color::DarkGray)),
        popup,
    );
}
