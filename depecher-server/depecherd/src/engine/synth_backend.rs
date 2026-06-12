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

    /// Maximum reference audio duration this backend accepts, in seconds.
    /// The backend caps `audio_prompt` to this length before encoding.
    /// `None` means no limit (e.g. mock backends, future backends with longer context).
    fn max_reference_secs(&self) -> Option<f32> {
        None
    }
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
        let url = std::env::var("DEPECHER_TTS_URL")
            .or_else(|_| std::env::var("FISH_SPEECH_URL"))
            .unwrap_or_else(|_| {
                "https://dpopsuev--depecher-tts-serve-chatterboxtts-tts.modal.run".into()
            });
        let token = std::env::var("DEPECHER_TTS_TOKEN").ok();
        let models_dir = std::env::var("RVC_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("training/models"));
        Self::new(url, token, models_dir)
    }

    /// Maximum reference audio this backend accepts — Chatterbox hard-truncates at 10s.
    pub fn max_reference_secs(&self) -> Option<f32> {
        Some(10.0)
    }

    /// Read optional per-model config from `<models_dir>/<model>/`.
    /// Reference audio is capped to `max_reference_secs()` before base64-encoding.
    fn model_config(&self, model: &str) -> (String, Option<String>) {
        let dir = self.models_dir.join(model);

        let lang = std::fs::read_to_string(dir.join("lang"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "ru".into());

        let audio_prompt = std::fs::read(dir.join("reference.wav")).ok().map(|bytes| {
            use base64::Engine as _;
            let capped = match self.max_reference_secs() {
                Some(max) => crate::wav::cap_wav(&bytes, max),
                None => bytes,
            };
            base64::engine::general_purpose::STANDARD.encode(&capped)
        });

        (lang, audio_prompt)
    }
}

#[async_trait::async_trait]
impl SynthBackend for ModalSynthBackend {
    fn max_reference_secs(&self) -> Option<f32> {
        self.max_reference_secs()
    }

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
        std::env::set_var("DEPECHER_TTS_URL", "http://example.com/tts");
        let b = ModalSynthBackend::from_env();
        assert_eq!(b.url, "http://example.com/tts");
    }

    #[test]
    fn modal_backend_caps_reference_at_10s() {
        let b = ModalSynthBackend::from_env();
        assert_eq!(b.max_reference_secs(), Some(10.0));
    }

    #[test]
    fn mock_backend_has_no_reference_cap() {
        let b = MockSynthBackend::instant();
        assert_eq!(b.max_reference_secs(), None);
    }

    #[test]
    fn cap_is_applied_inside_model_config() {
        let dir = tempfile::tempdir().unwrap();
        let model = "testmodel";
        let model_dir = dir.path().join(model);
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join("lang"), "en").unwrap();

        // Write a 20s WAV — should be capped to 10s.
        let n = 24_000usize * 20;
        let samples: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 24_000.0).sin() * 0.3)
            .collect();
        let wav_bytes = crate::wav::encode_wav(&samples, 24_000).unwrap();
        std::fs::write(model_dir.join("reference.wav"), &wav_bytes).unwrap();

        let b = ModalSynthBackend::new("http://x".to_string(), None, dir.path().to_path_buf());
        let (_lang, ap) = b.model_config(model);
        let ap_bytes = ap.expect("audio_prompt should be set");

        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&ap_bytes)
            .unwrap();
        let wav = depecher_analyse::decode_wav(&decoded).unwrap();
        let dur = wav.samples.len() as f32 / wav.sample_rate as f32;
        assert!(
            (dur - 10.0).abs() < 0.05,
            "model_config should cap to 10s, got {dur:.2}s"
        );
    }
}
