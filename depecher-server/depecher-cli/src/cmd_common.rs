use std::cmp::Reverse;
use std::path::PathBuf;
use std::process::Command;
use tracing::info;

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

pub fn synth_request(
    server: &str,
    text: &str,
    model: &str,
    voice: &str,
    speed: u32,
    dsp: bool,
    opts: serde_json::Value,
) -> Result<Vec<u8>, String> {
    synth_request_ext(
        server,
        text,
        model,
        voice,
        speed,
        dsp,
        opts,
        &serde_json::Map::new(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn synth_request_ext(
    server: &str,
    text: &str,
    model: &str,
    voice: &str,
    speed: u32,
    dsp: bool,
    opts: serde_json::Value,
    extra: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<u8>, String> {
    let mut body = serde_json::json!({
        "text":  text,
        "model": model,
        "voice": voice,
        "speed": speed,
        "dsp":   dsp,
        "opts":  opts,
    });
    for (k, v) in extra {
        body[k] = v.clone();
    }

    let t0 = std::time::Instant::now();
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

    let wav = resp
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|e| e.to_string())?;
    let rtt_ms = t0.elapsed().as_millis() as u64;

    // Record inference cost. Cache hits return in <150ms.
    let cache_hit = rtt_ms < 150;
    let audio_duration_sec = wav_duration_sec(&wav);
    crate::cost::record_inference(crate::cost::InferenceReceipt::new(
        text,
        audio_duration_sec,
        rtt_ms,
        cache_hit,
    ));

    Ok(wav)
}

/// Estimate audio duration from a WAV byte slice.
/// Returns 0.0 for unrecognised formats.
fn wav_duration_sec(wav: &[u8]) -> f64 {
    // Minimal WAV header parse: sample_rate at offset 24 (4 bytes LE),
    // num_channels at 22 (2 bytes LE), bits_per_sample at 34 (2 bytes LE),
    // data chunk size at 40 (4 bytes LE).
    if wav.len() < 44 {
        return 0.0;
    }
    let channels = u16::from_le_bytes([wav[22], wav[23]]) as f64;
    let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]) as f64;
    let bits = u16::from_le_bytes([wav[34], wav[35]]) as f64;
    let data_bytes = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as f64;
    if channels == 0.0 || sample_rate == 0.0 || bits == 0.0 {
        return 0.0;
    }
    data_bytes / (sample_rate * channels * (bits / 8.0))
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
    info!("⚠  No audio player found (tried paplay, afplay, mpv, aplay, ffplay)");
    tracing::info!("   File saved to: {}", path.display());
}

pub fn save_and_maybe_play(bytes: &[u8], out: Option<&PathBuf>, play: bool) -> PathBuf {
    let path = if let Some(p) = out {
        std::fs::write(p, bytes).unwrap();
        p.clone()
    } else {
        let mut tmp = std::env::temp_dir();
        tmp.push(format!(
            "depecherctl_{}.wav",
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

pub fn print_tune_results(results: &[(String, u8, String)]) {
    if results.is_empty() {
        return;
    }
    let mut sorted = results.to_vec();
    sorted.sort_by_key(|a| Reverse(a.1));
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
            tracing::info!("  ⚠  cannot read maquettes file {}: {e}", p.display());
            return default_maquettes();
        }
    };
    let entries: Vec<serde_json::Value> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            info!("⚠  invalid maquettes JSON: {e}");
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
    let path = std::env::temp_dir().join(format!(
        "depecherctl_maquette_{}.wav",
        m.name.replace(' ', "_")
    ));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}
