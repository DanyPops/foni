use std::path::PathBuf;
use std::process::Command;

pub fn data_dir() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".local/share"))
        .join("foni")
}

pub fn cache_dir() -> PathBuf {
    std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".cache"))
        .join("foni")
}

pub fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn dirs_next(var: &str) -> PathBuf {
    std::env::var(var).map(PathBuf::from).unwrap_or_default()
}

pub fn epoch_ymd(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let (s, m, h) = (secs % 60, (secs / 60) % 60, (secs / 3600) % 24);
    let mut rem = secs / 86400;
    let mut y = 1970u64;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let yd = if leap { 366 } else { 365 };
        if rem < yd {
            break;
        }
        rem -= yd;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let md = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 0u64;
    for &days in &md {
        if rem < days {
            break;
        }
        rem -= days;
        mo += 1;
    }
    (y, mo + 1, rem + 1, h, m, s)
}

pub fn synth_request(
    server: &str,
    text: &str,
    model: &str,
    voice: &str,
    speed: u32,
    dsp: bool,
    opts: serde_json::Value,
) -> Result<Vec<u8>, String> {
    let body = serde_json::json!({
        "text":  text,
        "model": model,
        "voice": voice,
        "speed": speed,
        "dsp":   dsp,
        "opts":  opts,
    });

    let resp = reqwest::blocking::Client::new()
        .post(format!("{server}/synthesize"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "HTTP {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        ));
    }

    resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
}

pub fn process_request(
    server: &str,
    wav: &[u8],
    opts: serde_json::Value,
) -> Result<Vec<u8>, String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let body = serde_json::json!({
        "audio_data": B64.encode(wav),
        "opts":       opts,
    });
    let resp = reqwest::blocking::Client::new()
        .post(format!("{server}/process"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .map_err(|e| format!("/process request: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "HTTP {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        ));
    }
    let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
    let b64 = data["audio_data"]
        .as_str()
        .ok_or("no audio_data in response")?;
    B64.decode(b64).map_err(|e| e.to_string())
}

pub fn get_json(server: &str, path: &str) -> Result<serde_json::Value, String> {
    reqwest::blocking::get(format!("{server}{path}"))
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())
}

pub fn wav_duration_secs(path: &std::path::Path) -> Option<f64> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 44 {
        return None;
    }
    let sr = u32::from_le_bytes(data[24..28].try_into().ok()?) as f64;
    let channels = u16::from_le_bytes(data[22..24].try_into().ok()?) as f64;
    let bits = u16::from_le_bytes(data[34..36].try_into().ok()?) as f64;
    let data_size = u32::from_le_bytes(data[40..44].try_into().ok()?) as f64;
    Some(data_size / (sr * channels * bits / 8.0))
}

pub fn play_wav(path: &std::path::Path) {
    for player in &["paplay", "afplay", "mpv", "aplay", "ffplay"] {
        if Command::new(player)
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return;
        }
    }
    if let Some(dur) = wav_duration_secs(path) {
        std::thread::sleep(std::time::Duration::from_secs_f64(dur));
    }
    eprintln!("⚠  No audio player found (tried paplay, afplay, mpv, aplay, ffplay)");
    eprintln!("   File saved to: {}", path.display());
}

pub fn save_and_maybe_play(bytes: &[u8], out: Option<&PathBuf>, play: bool) -> PathBuf {
    let path = if let Some(p) = out {
        std::fs::write(p, bytes).unwrap();
        p.clone()
    } else {
        let mut tmp = std::env::temp_dir();
        tmp.push(format!(
            "fonictl_{}.wav",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::write(&tmp, bytes).unwrap();
        tmp
    };

    if play {
        play_wav(&path);
    }
    path
}

pub fn stars(rating: Option<u8>) -> String {
    match rating {
        None => "     ".into(),
        Some(n) => format!("{}☆{}", "★".repeat(n as usize), "★".repeat(5 - n as usize)),
    }
}

pub fn opts_describe(opts: &serde_json::Value) -> String {
    let get = |k: &str| opts.get(k).and_then(|v| v.as_f64());
    let mut parts = Vec::new();
    if let Some(v) = get("rmsTargetLufs") {
        parts.push(format!("rms={v:.0}"));
    }
    if let Some(v) = get("compressionRatio") {
        parts.push(format!("comp={v:.1}"));
    }
    if let Some(v) = get("tiltLowDb") {
        parts.push(format!("lo={v:.0}"));
    }
    if let Some(v) = get("tiltHighDb") {
        parts.push(format!("hi={v:.0}"));
    }
    if let Some(v) = get("vibratoFreq") {
        parts.push(format!("vib_f={v:.0}"));
    }
    if let Some(v) = get("vibratoDepth") {
        parts.push(format!("vib={v:.4}"));
    }
    if let Some(v) = get("presenceDb") {
        parts.push(format!("pres={v:.1}"));
    }
    if let Some(v) = get("deEssDb") {
        parts.push(format!("deess={v:.0}"));
    }
    if let Some(v) = get("reverbMs") {
        parts.push(format!("rev={v:.0}ms"));
    }
    if parts.is_empty() {
        "defaults".into()
    } else {
        parts.join("  ")
    }
}

pub fn print_tune_results(results: &[(String, u8, String)]) {
    if results.is_empty() {
        return;
    }
    let mut sorted = results.to_vec();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n╬══ Results ══════════════════════════════════════════");
    for (name, score, note) in &sorted {
        let stars = format!(
            "{}{}",
            "★".repeat(*score as usize),
            "☆".repeat(5 - *score as usize)
        );
        let note_str = if note.is_empty() {
            String::new()
        } else {
            format!(" — {note}")
        };
        println!("  {stars}  {name}{note_str}");
    }
    println!("╚════════════════════════════════════════════");
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Maquette {
    pub name: String,
    pub opts: serde_json::Value,
}

impl Maquette {
    pub fn describe(&self) -> String {
        let o = &self.opts;
        let mut parts = vec![];
        if let Some(v) = o.get("rmsTargetLufs") {
            parts.push(format!("rms={:.0}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("compressionRatio") {
            parts.push(format!("comp={:.1}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("tiltLowDb") {
            parts.push(format!("lo={:.0}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("tiltHighDb") {
            parts.push(format!("hi={:.0}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("presenceDb") {
            parts.push(format!("pres={:.1}", v.as_f64().unwrap_or(0.0)));
        }
        if let Some(v) = o.get("deEssDb") {
            parts.push(format!("deess={:.0}", v.as_f64().unwrap_or(0.0)));
        }
        if parts.is_empty() {
            "defaults".into()
        } else {
            parts.join("  ")
        }
    }
}

pub fn load_maquettes(path: Option<&std::path::Path>) -> Vec<Maquette> {
    let Some(p) = path else {
        return default_maquettes();
    };
    let raw = match std::fs::read_to_string(p) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  ⚠  cannot read maquettes file {}: {e}", p.display());
            return default_maquettes();
        }
    };
    let entries: Vec<serde_json::Value> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  ⚠  invalid maquettes JSON: {e}");
            return default_maquettes();
        }
    };
    entries
        .into_iter()
        .filter_map(|v| {
            Some(Maquette {
                name: v["name"].as_str()?.to_owned(),
                opts: v["opts"].clone(),
            })
        })
        .collect()
}

pub fn default_maquettes() -> Vec<Maquette> {
    vec![
        Maquette {
            name: "baseline".into(),
            opts: serde_json::json!({
            "rmsTargetLufs": -8, "compressionRatio": 4, "compressionMakeupDb": 5,
            "tiltLowDb": 10, "tiltHighDb": -8, "vibratoFreq": 6, "vibratoDepth": 0.003 }),
        },
        Maquette {
            name: "warm".into(),
            opts: serde_json::json!({
            "rmsTargetLufs": -8, "compressionRatio": 3, "compressionMakeupDb": 4,
            "tiltLowDb": 14, "tiltHighDb": -10, "vibratoFreq": 6, "vibratoDepth": 0.003 }),
        },
        Maquette {
            name: "punchy".into(),
            opts: serde_json::json!({
            "rmsTargetLufs": -6, "compressionRatio": 6, "compressionMakeupDb": 6,
            "tiltLowDb": 8,  "tiltHighDb": -6,  "vibratoFreq": 5, "vibratoDepth": 0.002 }),
        },
        Maquette {
            name: "bright".into(),
            opts: serde_json::json!({
            "rmsTargetLufs": -9, "compressionRatio": 3, "compressionMakeupDb": 3,
            "tiltLowDb": 6,  "tiltHighDb": -4,  "presenceDb": 2.0, "deEssDb": 3.0,
            "vibratoFreq": 6, "vibratoDepth": 0.003 }),
        },
    ]
}

pub fn render_maquette(
    server: &str,
    text: &str,
    model: &str,
    m: &Maquette,
) -> Result<PathBuf, String> {
    let bytes = synth_request(server, text, model, "ru", 150, true, m.opts.clone())?;
    let path =
        std::env::temp_dir().join(format!("fonictl_maquette_{}.wav", m.name.replace(' ', "_")));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn mixer_print_tracks(tracks: &[Track]) {
    println!();
    println!(
        "  {:>3}  {:<20}  {:<7}  {}",
        "#", "Label", "Rating", "Config"
    );
    println!("  {}", "─".repeat(78));
    for (i, t) in tracks.iter().enumerate() {
        let dot = if t.path.exists() { "●" } else { "◦" };
        let crown = if t.winner { " ✪" } else { "  " };
        let rating = stars(t.rating);
        println!(
            "  {dot}{crown}{:>2}  {:<20}  {rating}  {}",
            i + 1,
            t.label,
            t.desc
        );
        if let Some(ref n) = t.note {
            println!("        └ {n}");
        }
    }
    println!();
}

pub struct Track {
    pub label: String,
    pub desc: String,
    pub path: std::path::PathBuf,
    pub opts: serde_json::Value,
    pub rating: Option<u8>,
    pub note: Option<String>,
    pub winner: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct TrackRecord {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub opts: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_pct: Option<f32>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct AiSuggestion {
    pub label: String,
    pub rationale: String,
    pub opts: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct MixerSession {
    pub phrase: String,
    pub model: String,
    pub saved_at: String,
    pub tracks: Vec<TrackRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ai_suggestions: Vec<AiSuggestion>,
}

impl MixerSession {
    pub fn path() -> std::path::PathBuf {
        let base = std::env::var("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let mut p = dirs_next("HOME");
                p.push(".local/share");
                p
            });
        let dir = base.join("foni");
        std::fs::create_dir_all(&dir).ok();
        dir.join("mixer-session.json")
    }

    pub fn save(&self) {
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(Self::path(), json).ok();
    }

    pub fn load() -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(Self::path()).ok()?).ok()
    }

    pub fn now() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (y, mo, d, h, mi, s) = epoch_ymd(secs);
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }
}

pub fn save_session(tracks: &[Track], phrase: &str, model: &str) {
    let session = MixerSession {
        phrase: phrase.to_string(),
        model: model.to_string(),
        saved_at: MixerSession::now(),
        tracks: tracks
            .iter()
            .map(|t| TrackRecord {
                label: t.label.clone(),
                rating: t.rating,
                winner: if t.winner { Some(true) } else { None },
                note: t.note.clone(),
                opts: t.opts.clone(),
                gap_pct: None,
            })
            .collect(),
        ai_suggestions: Vec::new(),
    };
    session.save();
}

pub const MIXER_HELP: &str = r#"
  Playback:
    N            play track N
    r            replay last
    cmp A B      play A then B  ('ref' = Sidorovich original)

  Rating (written to ~/.local/share/foni/mixer-session.json):
    rate N SCORE note...   rate track N (1-5) with optional note
    winner N               mark track N as winner (auto-saves)
    note N TEXT            add/replace note on track N
    ai                     load AI suggestions as pending tracks
    save                   write session file now

  DSP scratchpad:
    opts                   show current scratchpad opts
    set KEY VAL            tweak a param  (e.g. set vibratoDepth 0)
    reset                  reset to defaults
    fork N                 copy track N opts to scratchpad
    render NAME            render scratchpad as new track

  Tracks:
    ls                     list all tracks
    drop N                 remove track N
    analyse N              acoustic metrics for track N
    phrase TEXT            change phrase (clears rendered tracks)

    q / quit               save session and exit
"#;
