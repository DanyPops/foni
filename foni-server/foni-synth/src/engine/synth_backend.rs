//! SynthBackend — Strategy pattern for TTS synthesis.
//!
//! The WS handler calls `SynthBackend::synthesize()` directly instead of
//! routing through `localhost:5050/synthesize`. This eliminates the self-call
//! deadlock (FON-TSK-197) and makes synthesis injectable for tests.
//!
//! Implementations:
//! - `ModalSynthBackend` — calls the Chatterbox Modal endpoint directly.
//! - `MockSynthBackend`  — returns a sine WAV after an optional delay (tests).

use std::f32::consts::PI;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Synthesize text to raw WAV bytes.
#[async_trait::async_trait]
pub trait SynthBackend: Send + Sync {
    async fn synthesize(&self, text: &str, model: &str) -> Result<Vec<u8>, String>;
}

// ── ModalSynthBackend ─────────────────────────────────────────────────────────

/// Calls the Chatterbox TTS endpoint on Modal directly.
/// Shared `reqwest::Client` — no connection pool leak per call.
///
/// Per-model voice cloning: if `<models_dir>/<model>/reference.wav` exists,
/// it is base64-encoded and forwarded as `audio_prompt` for zero-shot cloning.
/// Language defaults to `ru`; override by placing a `lang` file (e.g. `en`)
/// alongside `reference.wav`.
pub struct ModalSynthBackend {
    url: String,
    token: Option<String>,
    models_dir: PathBuf,
    client: reqwest::Client,
}

impl ModalSynthBackend {
    pub fn new(url: impl Into<String>, token: Option<String>, models_dir: PathBuf) -> Self {
        Self {
            url: url.into(),
            token,
            models_dir,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest client"),
        }
    }

    pub fn from_env() -> Self {
        let url = std::env::var("FONI_TTS_URL")
            .or_else(|_| std::env::var("FISH_SPEECH_URL"))
            .unwrap_or_else(|_| {
                "https://dpopsuev--foni-tts-serve-chatterboxtts-tts.modal.run".into()
            });
        let token = std::env::var("FONI_TTS_TOKEN").ok();
        let models_dir = std::env::var("RVC_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("training/models"));
        Self::new(url, token, models_dir)
    }

    /// Read optional per-model config from `<models_dir>/<model>/`.
    fn model_config(&self, model: &str) -> (String, Option<String>) {
        let dir = self.models_dir.join(model);

        let lang = std::fs::read_to_string(dir.join("lang"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "ru".into());

        let audio_prompt = std::fs::read(dir.join("reference.wav")).ok().map(|bytes| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        });

        (lang, audio_prompt)
    }
}

#[async_trait::async_trait]
impl SynthBackend for ModalSynthBackend {
    async fn synthesize(&self, text: &str, model: &str) -> Result<Vec<u8>, String> {
        let (lang, audio_prompt) = self.model_config(model);
        let mut body = serde_json::json!({"text": text, "language": lang});

        if let Some(ref ap) = audio_prompt {
            body["audio_prompt"] = serde_json::json!(ap);
            tracing::debug!(model, "synth: using reference audio for voice cloning");
        }
        if let Some(token) = &self.token {
            body["token"] = serde_json::json!(token);
        }
        let resp = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("TTS request: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("TTS HTTP {}", resp.status()));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| e.to_string())
    }
}

// ── MockSynthBackend ──────────────────────────────────────────────────────────

/// Returns a sine WAV immediately (or after `delay`). Zero network calls.
/// Used in integration tests so synthesis never self-calls the server.
pub struct MockSynthBackend {
    delay: Option<Duration>,
}

impl MockSynthBackend {
    pub fn instant() -> Self {
        Self { delay: None }
    }

    pub fn with_delay(ms: u64) -> Self {
        Self {
            delay: Some(Duration::from_millis(ms)),
        }
    }
}

#[async_trait::async_trait]
impl SynthBackend for MockSynthBackend {
    async fn synthesize(&self, _text: &str, _model: &str) -> Result<Vec<u8>, String> {
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        Ok(sine_wav(440.0, 0.1, 24_000))
    }
}

fn sine_wav(freq: f32, secs: f32, rate: u32) -> Vec<u8> {
    let n = (rate as f32 * secs) as usize;
    let samples: Vec<f32> = (0..n)
        .map(|i| (2.0 * PI * freq * i as f32 / rate as f32).sin() * 0.3)
        .collect();
    crate::wav::encode_wav(&samples, rate).expect("infallible")
}

// ── Arc wrapper ───────────────────────────────────────────────────────────────

pub type SharedSynth = Arc<dyn SynthBackend>;

pub fn modal_backend() -> SharedSynth {
    Arc::new(ModalSynthBackend::from_env())
}

pub fn mock_backend() -> SharedSynth {
    Arc::new(MockSynthBackend::instant())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_valid_wav() {
        let b = MockSynthBackend::instant();
        let wav = b.synthesize("test", "sidorovich").await.unwrap();
        assert!(wav.len() > 44, "should be more than a WAV header");
        assert_eq!(&wav[0..4], b"RIFF");
    }

    #[tokio::test]
    async fn mock_with_delay_takes_at_least_that_long() {
        let b = MockSynthBackend::with_delay(50);
        let t = std::time::Instant::now();
        b.synthesize("x", "m").await.unwrap();
        assert!(t.elapsed().as_millis() >= 50);
    }

    #[tokio::test]
    async fn modal_backend_reads_foni_tts_url_env() {
        std::env::set_var("FONI_TTS_URL", "http://example.com/tts");
        let b = ModalSynthBackend::from_env();
        assert_eq!(b.url, "http://example.com/tts");
    }
}
