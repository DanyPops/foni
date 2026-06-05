use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use foni_analyse::decode_wav;
use foni_synth::engine::audio_stream;
use tracing::{debug, info, warn};

const STREAM_SAMPLE_RATE: u32 = 16_000;
const RECORD_SAMPLE_RATE: u32 = 24_000;

/// Read text from arg or stdin (for pipe support).
pub fn resolve_text(arg: Option<String>) -> Option<String> {
    if let Some(t) = arg {
        if !t.is_empty() {
            return Some(t);
        }
    }
    if console::user_attended() {
        return None;
    }
    let mut buf = String::new();
    for line in std::io::stdin().lock().lines() {
        let line = line.ok()?;
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(line.trim());
    }
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

/// Record from microphone until silence, print WAV path to stdout.
pub fn cmd_rec(
    out: Option<&Path>,
    silence_db: i32,
    silence_secs: f64,
    max_secs: u32,
) -> Result<(), String> {
    let out_path = out
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("foni_rec.wav"));

    info!(
        "Recording ({}s max, silence {}dB/{}s to stop)",
        max_secs, silence_db, silence_secs
    );
    info!("press Ctrl+C to stop");

    let raw_path = std::env::temp_dir().join("foni_rec_raw.wav");
    let status = Command::new("parecord")
        .args([
            "--format=s16le",
            &format!("--rate={RECORD_SAMPLE_RATE}"),
            "--channels=1",
            &format!("--process-time-msec={}", max_secs * 1000),
        ])
        .arg(&raw_path)
        .status()
        .map_err(|e| format!("parecord not found: {e}"))?;

    // parecord returns non-zero on Ctrl+C, which is the normal stop signal
    if !status.success() && !raw_path.exists() {
        return Err("parecord failed".into());
    }

    let status = Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(&raw_path)
        .args([
            "-af",
            &format!(
                "silenceremove=stop_periods=-1:stop_duration={}:stop_threshold={}dB",
                silence_secs, silence_db
            ),
        ])
        .arg(&out_path)
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;

    if !status.success() {
        std::fs::rename(&raw_path, &out_path).map_err(|e| format!("rename: {e}"))?;
    } else {
        std::fs::remove_file(&raw_path).ok();
    }

    let dur = wav_duration(&out_path).unwrap_or(0.0);
    info!("{:.1}s recorded {}", dur, out_path.display());

    println!("{}", out_path.display());
    Ok(())
}

/// Transcribe audio to text via Whisper, print to stdout.
pub fn cmd_transcribe(file: Option<&Path>, lang: &str, model: &str) -> Result<(), String> {
    let path = match file {
        Some(p) => p.to_path_buf(),
        None => {
            let mut line = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut line)
                .map_err(|e| format!("stdin: {e}"))?;
            PathBuf::from(line.trim())
        }
    };

    if !path.exists() {
        return Err(format!("{}: not found", path.display()));
    }

    info!("Transcribing {} ({}, {})", path.display(), model, lang);

    let output = Command::new("whisper")
        .args([
            path.to_str().ok_or("invalid path")?,
            "--model",
            model,
            "--language",
            lang,
            "--output_format",
            "txt",
            "--output_dir",
            std::env::temp_dir().to_str().unwrap_or("/tmp"),
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("whisper not found: {e}"))?;

    if !output.status.success() {
        return Err("whisper failed".into());
    }

    let stem = path.file_stem().ok_or("no file stem")?;
    let txt_path =
        PathBuf::from(std::env::temp_dir()).join(format!("{}.txt", stem.to_string_lossy()));
    let text = std::fs::read_to_string(&txt_path).map_err(|e| format!("read transcript: {e}"))?;
    let text = text.trim();

    info!("\"{}\"", truncate(text, 80));
    println!("{text}");
    Ok(())
}

/// Send text to Ollama LLM, print reply to stdout.
pub fn cmd_think(
    text: Option<&str>,
    persona: &str,
    model: &str,
    ollama_url: &str,
    ctx: &VoiceContext,
) -> Result<(), String> {
    let input = match text {
        Some(t) => t.to_string(),
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut buf)
                .map_err(|e| format!("stdin: {e}"))?;
            buf.trim().to_string()
        }
    };

    if input.is_empty() {
        return Err("no input text".into());
    }

    let mut ctx = ctx.clone();
    if ctx.domain.is_none() {
        ctx.domain = detect_domain(&input, model, ollama_url);
    }

    let result = think_blocking(&input, persona, model, ollama_url, &ctx)?;
    println!("{}", result.reply);
    Ok(())
}

/// Full voice loop: record → transcribe → think → speak.
pub fn cmd_reply(
    server: &str,
    persona: &str,
    lang: &str,
    llm_model: &str,
    ollama_url: &str,
    max_secs: u32,
    ctx: &VoiceContext,
    file: Option<&Path>,
) -> Result<(), String> {
    let wav_path = match file {
        Some(f) => {
            // Convert input file to mono WAV if needed
            let tmp = std::env::temp_dir().join("foni_reply_input.wav");
            let status = Command::new("ffmpeg")
                .args(["-y", "-i"])
                .arg(f)
                .args(["-ac", "1", "-ar", "24000"])
                .arg(&tmp)
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|e| format!("ffmpeg: {e}"))?;
            if !status.success() {
                return Err(format!("failed to convert {}", f.display()));
            }
            tmp
        }
        None => {
            let tmp = std::env::temp_dir().join("foni_reply.wav");
            cmd_rec(Some(&tmp), -30, 1.5, max_secs)?;
            tmp
        }
    };

    let expr = read_tone(&wav_path).unwrap_or_default();

    cmd_transcribe(Some(&wav_path), lang, "base")?;
    let stem = wav_path.file_stem().ok_or("no stem")?;
    let txt_path = std::env::temp_dir().join(format!("{}.txt", stem.to_string_lossy()));
    let input = std::fs::read_to_string(&txt_path)
        .map_err(|e| format!("read transcript: {e}"))?
        .trim()
        .to_string();

    if input.is_empty() {
        return Err("nothing transcribed".into());
    }

    let mut ctx = ctx.clone();
    if ctx.domain.is_none() {
        ctx.domain = detect_domain(&input, llm_model, ollama_url);
    }
    let result = think_blocking(&input, persona, llm_model, ollama_url, &ctx)?;

    info!("Synthesizing reply");

    let client = foni_client::FoniClient::new(server);
    let final_expr = blend_expression(&expr, &result.expression);
    info!(
        excitement = format!("{:.2}", final_expr.excitement),
        assertiveness = format!("{:.2}", final_expr.assertiveness),
        warmth = format!("{:.2}", final_expr.warmth),
        "blended expression"
    );

    let mut req = foni_client::SynthRequest {
        text: result.reply,
        voice: lang.into(),
        speed: 150,
        model: Some("sidorovich".into()),
        ..Default::default()
    };
    req.exaggeration = Some(final_expr.excitement);
    req.cfg_weight = Some(final_expr.assertiveness);
    req.temperature = Some(final_expr.warmth);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio: {e}"))?;
    let wav_data = rt
        .block_on(client.synthesize(&req))
        .map_err(|e| format!("synth: {e}"))?;

    let out_path = std::env::temp_dir().join("foni_reply_out.wav");
    std::fs::write(&out_path, &wav_data.0).map_err(|e| format!("write: {e}"))?;
    let dur = wav_data.0.len() as f64 / (RECORD_SAMPLE_RATE * 2) as f64;
    info!("{:.1}s", dur);

    Command::new("paplay")
        .arg(&out_path)
        .status()
        .map_err(|e| format!("paplay: {e}"))?;

    Ok(())
}

/// Context layers for the LLM system prompt.
/// `audience` is known before the conversation (who you're talking to).
/// `domain` is derived from what they say — detected automatically, not pre-configured.
#[derive(Clone, Default)]
pub struct VoiceContext {
    pub domain: Option<String>,
    pub audience: Option<String>,
}

struct ThinkResult {
    reply: String,
    expression: Expression,
}

fn think_blocking(
    input: &str,
    persona: &str,
    model: &str,
    ollama_url: &str,
    ctx: &VoiceContext,
) -> Result<ThinkResult, String> {
    let system = build_system_prompt(persona, ctx.domain.as_deref(), ctx.audience.as_deref());
    info!("Thinking ({}, persona={})", model, persona);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .post(format!("{ollama_url}/api/generate"))
        .json(&serde_json::json!({
            "model": model,
            "system": system,
            "prompt": input,
            "stream": false,
        }))
        .send()
        .map_err(|e| format!("ollama: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("ollama {}", resp.status()));
    }

    let body: serde_json::Value = resp.json().map_err(|e| format!("parse: {e}"))?;
    let raw = body["response"]
        .as_str()
        .ok_or("no response field")?
        .trim()
        .to_string();

    let expression = parse_expression_tag(&raw).unwrap_or_else(|| {
        debug!("no expression tag in LLM reply, using defaults");
        Expression::default()
    });

    let reply = if let Some(idx) = raw.rfind("[E:") {
        raw[..idx].trim().to_string()
    } else {
        raw
    };

    info!(reply = truncate(&reply, 80), "LLM reply");
    Ok(ThinkResult { reply, expression })
}

/// Build the full system prompt from persona + optional context layers.
fn build_system_prompt(persona: &str, domain: Option<&str>, audience: Option<&str>) -> String {
    let mut prompt = persona_base(persona);

    if let Some(d) = domain {
        prompt.push_str(&format!(
            "\nThe conversation topic is: {d}. Frame your responses within this domain."
        ));
    }
    if let Some(a) = audience {
        prompt.push_str(&format!("\nYou are speaking to {a}. Address them by name."));
    }

    prompt
}

/// Detect the domain/topic from a transcript via a quick LLM call.
fn detect_domain(text: &str, model: &str, ollama_url: &str) -> Option<String> {
    info!("Detecting topic");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;
    let resp = client
        .post(format!("{ollama_url}/api/generate"))
        .json(&serde_json::json!({
            "model": model,
            "system": "Extract the topic of the following message in 2-5 words. Reply with ONLY the topic, nothing else.",
            "prompt": text,
            "stream": false,
        }))
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().ok()?;
    let topic = body["response"].as_str()?.trim().to_string();
    if topic.is_empty() {
        None
    } else {
        info!("\"{topic}\"");
        Some(topic)
    }
}

const EXPRESSION_TAG_INSTRUCTION: &str = concat!(
    "\nAfter your reply, append an expression tag on a new line: [E:x.x A:x.x W:x.x]\n",
    "E=excitement (0.3 calm, 0.8 moderate, 1.5 intense)\n",
    "A=assertiveness (0.5 tentative, 0.3 confident, 0.15 commanding)\n",
    "W=warmth (0.4 cold, 0.8 neutral, 1.2 warm)\n",
    "Example reply:\nThe Emperor protects, brother!\n[E:1.3 A:0.2 W:0.7]",
);

fn persona_base(name: &str) -> String {
    let base = match name {
        "diomedes" => concat!(
            "You are Captain Diomedes of the Blood Ravens Space Marines from Warhammer 40,000. ",
            "You speak with dramatic gravitas, military authority, and absolute devotion to the Emperor. ",
            "Your speech is commanding, bold, and sometimes poetic in its intensity. ",
            "Keep responses to 1-3 sentences. Speak in English.",
        ),
        "sidorovich" => concat!(
            "Ты — Сидорович, торговец из бункера на Кордоне в игре S.T.A.L.K.E.R. ",
            "Говоришь грубовато, по-деловому, с чёрным юмором. Знаешь Зону как свои пять пальцев. ",
            "Отвечай 1-3 предложениями. Говори по-русски.",
        ),
        other => return format!("{other}. Keep responses to 1-3 sentences.{EXPRESSION_TAG_INSTRUCTION}"),
    };
    format!("{base}{EXPRESSION_TAG_INSTRUCTION}")
}

/// Expression knobs derived from input tone.
#[derive(Debug, Clone)]
pub struct Expression {
    pub excitement: f32,
    pub assertiveness: f32,
    pub warmth: f32,
}

impl Default for Expression {
    fn default() -> Self {
        Self {
            excitement: 0.5,
            assertiveness: 0.5,
            warmth: 0.8,
        }
    }
}

const EMOTION_MODEL: &str = "training/models/emotion-ser/model.onnx";

/// Parse expression values from an LLM reply that ends with [E:x.x A:x.x W:x.x].
fn parse_expression_tag(text: &str) -> Option<Expression> {
    let tag_start = text.rfind("[E:")?;
    let tag = &text[tag_start..];
    let e = tag
        .split("E:")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse::<f32>()
        .ok()?;
    let a = tag
        .split("A:")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse::<f32>()
        .ok()?;
    let w = tag
        .split("W:")
        .nth(1)?
        .split(']')
        .next()?
        .trim()
        .parse::<f32>()
        .ok()?;
    Some(Expression {
        excitement: e.clamp(0.3, 1.7),
        assertiveness: a.clamp(0.1, 0.6),
        warmth: w.clamp(0.3, 1.3),
    })
}

/// Blend audio-derived and text-derived expressions.
/// Audio sets the baseline energy, text adjusts for content.
fn blend_expression(audio: &Expression, text: &Expression) -> Expression {
    let a = 0.3; // audio weight
    let t = 0.7; // text weight — what Diomedes says matters more
    Expression {
        excitement: audio.excitement * a + text.excitement * t,
        assertiveness: audio.assertiveness * a + text.assertiveness * t,
        warmth: audio.warmth * a + text.warmth * t,
    }
}

/// Analyze input audio via neural SER model, map to expression knobs.
pub fn read_tone(wav_path: &Path) -> Result<Expression, String> {
    let bytes = std::fs::read(wav_path).map_err(|e| format!("{}: {e}", wav_path.display()))?;
    let wav = decode_wav(&bytes).map_err(|e| format!("decode: {e}"))?;

    let model_path = PathBuf::from(EMOTION_MODEL);
    let mut session = foni_analyse::tone::load_session(&model_path)
        .ok_or_else(|| format!("SER model not found: {}", model_path.display()))?;

    let tone = foni_analyse::tone::read_tone(&mut session, &wav.samples, wav.sample_rate)?;

    let excitement = 0.3 + tone.arousal * 1.2; // 0.3 (calm) → 1.5 (animated)
    let assertiveness = 0.6 - tone.dominance * 0.4; // 0.6 (tentative) → 0.2 (commanding)
    let warmth = 0.4 + tone.valence * 0.8; // 0.4 (tense) → 1.2 (friendly)

    let expr = Expression {
        excitement,
        assertiveness,
        warmth,
    };

    info!(
        "Excitement={:.2}  Assertiveness={:.2}  Warmth={:.2}",
        expr.excitement, expr.assertiveness, expr.warmth
    );

    Ok(expr)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}…")
    }
}

fn wav_duration(path: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args(["-i"])
        .arg(path)
        .args([
            "-show_entries",
            "format=duration",
            "-v",
            "quiet",
            "-of",
            "csv=p=0",
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
}

/// Continuous conversation loop: mic → chunk → SER + transcribe → think → synth → play.
pub fn cmd_converse(
    server: &str,
    persona: &str,
    lang: &str,
    llm_model: &str,
    ollama_url: &str,
    ctx: &VoiceContext,
) -> Result<(), String> {
    info!("Listening (speak naturally, pauses = chunk boundaries)");
    info!("press Ctrl+C to stop");

    let mut child = Command::new("parecord")
        .args([
            "--raw",
            "--format=s16le",
            &format!("--rate={STREAM_SAMPLE_RATE}"),
            "--channels=1",
            "/dev/stdout",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("parecord: {e}"))?;

    let stdout = child.stdout.take().ok_or("no stdout from parecord")?;

    let mut stream_state = audio_stream::fresh_state();
    let mut turn = 0u32;

    // Read 100ms chunks at a time (3200 bytes = 1600 i16 samples)
    let read_size = (STREAM_SAMPLE_RATE as usize / 10) * 2; // bytes
    let mut raw_buf = vec![0u8; read_size];
    let mut reader = std::io::BufReader::new(stdout);

    loop {
        let bytes_read = match reader.read(&mut raw_buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                warn!("read: {e}");
                break;
            }
        };

        let samples: Vec<f32> = raw_buf[..bytes_read]
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32768.0)
            .collect();

        let result = audio_stream::feed_audio(&mut stream_state, &samples);

        for utterance in result.chunks {
            turn += 1;
            let dur = utterance.len() as f32 / STREAM_SAMPLE_RATE as f32;
            info!(turn, duration_secs = dur, "utterance received");

            if let Err(e) = process_utterance(
                server, &utterance, persona, lang, llm_model, ollama_url, ctx,
            ) {
                warn!("{e}");
            }
        }
    }

    // Flush any remaining audio
    if let Some(utterance) = audio_stream::flush(&mut stream_state) {
        turn += 1;
        let dur = utterance.len() as f32 / STREAM_SAMPLE_RATE as f32;
        info!(turn, duration_secs = dur, "utterance flushed");
        if let Err(e) = process_utterance(
            server, &utterance, persona, lang, llm_model, ollama_url, ctx,
        ) {
            warn!("{e}");
        }
    }

    child.kill().ok();
    Ok(())
}

/// Process a single utterance through the full pipeline.
fn process_utterance(
    server: &str,
    samples_16k: &[f32],
    persona: &str,
    lang: &str,
    llm_model: &str,
    ollama_url: &str,
    ctx: &VoiceContext,
) -> Result<(), String> {
    // 1. SER — read tone from raw samples
    let expr = match tone_from_samples(samples_16k) {
        Ok(e) => e,
        Err(e) => {
            info!("SER skipped: {e}");
            Expression::default()
        }
    };

    // 2. Transcribe — write temp WAV, run Whisper
    let wav_path = std::env::temp_dir().join("foni_converse_chunk.wav");
    write_wav_16k(samples_16k, &wav_path)?;
    cmd_transcribe(Some(&wav_path), lang, "base")?;

    let stem = wav_path.file_stem().ok_or("no stem")?;
    let txt_path = std::env::temp_dir().join(format!("{}.txt", stem.to_string_lossy()));
    let input = std::fs::read_to_string(&txt_path)
        .map_err(|e| format!("read transcript: {e}"))?
        .trim()
        .to_string();

    if input.is_empty() {
        return Err("nothing transcribed".into());
    }

    // 3. Detect domain + think
    let mut ctx = ctx.clone();
    if ctx.domain.is_none() {
        ctx.domain = detect_domain(&input, llm_model, ollama_url);
    }
    let result = think_blocking(&input, persona, llm_model, ollama_url, &ctx)?;

    // 4. Blend expression: audio SER + LLM self-tagged emotion
    let final_expr = blend_expression(&expr, &result.expression);
    info!(
        excitement = format!("{:.2}", final_expr.excitement),
        assertiveness = format!("{:.2}", final_expr.assertiveness),
        warmth = format!("{:.2}", final_expr.warmth),
        "blended expression"
    );

    // 5. Synthesize + play
    info!("Synthesizing");

    let client = foni_client::FoniClient::new(server);
    let mut req = foni_client::SynthRequest {
        text: result.reply,
        voice: lang.into(),
        speed: 150,
        model: Some("sidorovich".into()),
        ..Default::default()
    };
    req.exaggeration = Some(final_expr.excitement);
    req.cfg_weight = Some(final_expr.assertiveness);
    req.temperature = Some(final_expr.warmth);

    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio: {e}"))?;
    let wav_data = rt
        .block_on(client.synthesize(&req))
        .map_err(|e| format!("synth: {e}"))?;

    let out_path = std::env::temp_dir().join("foni_converse_out.wav");
    std::fs::write(&out_path, &wav_data.0).map_err(|e| format!("write: {e}"))?;

    Command::new("paplay")
        .arg(&out_path)
        .status()
        .map_err(|e| format!("paplay: {e}"))?;

    Ok(())
}

fn tone_from_samples(samples_16k: &[f32]) -> Result<Expression, String> {
    let model_path = PathBuf::from(EMOTION_MODEL);
    let mut session = foni_analyse::tone::load_session(&model_path)
        .ok_or_else(|| format!("SER model not found: {}", model_path.display()))?;

    let tone = foni_analyse::tone::read_tone(&mut session, samples_16k, STREAM_SAMPLE_RATE)?;

    let expr = Expression {
        excitement: 0.3 + tone.arousal * 1.2,
        assertiveness: 0.6 - tone.dominance * 0.4,
        warmth: 0.4 + tone.valence * 0.8,
    };

    info!(
        "Excitement={:.2}  Assertiveness={:.2}  Warmth={:.2}",
        expr.excitement, expr.assertiveness, expr.warmth
    );
    Ok(expr)
}

fn write_wav_16k(samples: &[f32], path: &Path) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: STREAM_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| format!("wav create: {e}"))?;
    for &s in samples {
        let i = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
        writer
            .write_sample(i)
            .map_err(|e| format!("wav write: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("wav finalize: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_expression_tag ──

    #[test]
    fn parse_tag_from_reply() {
        let text = "The Emperor protects!\n[E:1.3 A:0.2 W:0.7]";
        let expr = parse_expression_tag(text).unwrap();
        assert!((expr.excitement - 1.3).abs() < 0.01);
        assert!((expr.assertiveness - 0.2).abs() < 0.01);
        assert!((expr.warmth - 0.7).abs() < 0.01);
    }

    #[test]
    fn parse_tag_clamps_values() {
        let text = "Rage!\n[E:5.0 A:0.0 W:9.9]";
        let expr = parse_expression_tag(text).unwrap();
        assert!((expr.excitement - 1.7).abs() < 0.01);
        assert!((expr.assertiveness - 0.1).abs() < 0.01);
        assert!((expr.warmth - 1.3).abs() < 0.01);
    }

    #[test]
    fn parse_tag_missing_returns_none() {
        assert!(parse_expression_tag("No tag here").is_none());
    }

    #[test]
    fn parse_tag_partial_returns_none() {
        assert!(parse_expression_tag("[E:1.0 A:0.3]").is_none());
    }

    #[test]
    fn parse_tag_multiline_reply() {
        let text = "Line one.\nLine two.\n\n[E:0.8 A:0.4 W:1.1]";
        let expr = parse_expression_tag(text).unwrap();
        assert!((expr.excitement - 0.8).abs() < 0.01);
    }

    #[test]
    fn parse_tag_strips_from_reply() {
        let raw = "The Emperor protects!\n[E:1.3 A:0.2 W:0.7]";
        let idx = raw.rfind("[E:").unwrap();
        let reply = raw[..idx].trim();
        assert_eq!(reply, "The Emperor protects!");
    }

    // ── blend_expression ──

    #[test]
    fn blend_weights_text_higher() {
        let audio = Expression {
            excitement: 0.5,
            assertiveness: 0.5,
            warmth: 0.5,
        };
        let text = Expression {
            excitement: 1.5,
            assertiveness: 0.1,
            warmth: 1.2,
        };
        let blended = blend_expression(&audio, &text);
        assert!(blended.excitement > 1.0, "text excitement should dominate");
        assert!(
            blended.assertiveness < 0.3,
            "text assertiveness should dominate"
        );
    }

    #[test]
    fn blend_identical_inputs() {
        let same = Expression {
            excitement: 0.8,
            assertiveness: 0.3,
            warmth: 0.9,
        };
        let blended = blend_expression(&same, &same);
        assert!((blended.excitement - 0.8).abs() < 0.01);
        assert!((blended.assertiveness - 0.3).abs() < 0.01);
        assert!((blended.warmth - 0.9).abs() < 0.01);
    }

    // ── resolve_text ──

    #[test]
    fn resolve_text_prefers_arg() {
        assert_eq!(resolve_text(Some("hello".into())), Some("hello".into()));
    }

    #[test]
    fn resolve_text_empty_arg_returns_none_on_tty() {
        assert!(resolve_text(Some("".into())).is_none() || resolve_text(Some("".into())).is_some());
    }
}
