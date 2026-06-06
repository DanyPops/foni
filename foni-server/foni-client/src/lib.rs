//! foni-client — typed async HTTP client for foni-synth.
//!
//! Every endpoint has a dedicated method returning concrete types.
//! `WavData` unifies the encoding split: /synthesize and /convert return
//! raw audio/wav bytes; /process and /analyse use base64 JSON.
//!
//! All methods are async. Construct one `FoniClient` and share it (Clone).

pub mod error;
pub mod types;

pub use error::FoniError;
pub use types::{
    AnalyseRequest, AnalyseResponse, BreathResponse, ConvertRequest, ModelsResponse,
    ProcessResponse, RvcParams, SynthRequest, WavData, WireOpts,
};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::time::Duration;

pub type Result<T> = std::result::Result<T, FoniError>;

/// Async HTTP client for foni-synth. Cheap to clone — shares the connection pool.
#[derive(Clone)]
pub struct FoniClient {
    base: String,
    http: reqwest::Client,
}

impl FoniClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base: base_url.into().trim_end_matches('/').to_owned(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(180))
                .build()
                .expect("infallible: no TLS config"),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// Returns true when the server responds to GET /params within 2 s.
    pub async fn is_available(&self) -> bool {
        self.http
            .get(format!("{}/params", self.base))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    // ── Audio synthesis ───────────────────────────────────────────────────────

    /// POST /synthesize — text → TTS → DSP → WAV.
    /// Returns raw WAV bytes (audio/wav).
    pub async fn synthesize(&self, req: &SynthRequest) -> Result<WavData> {
        let resp = self
            .http
            .post(format!("{}/synthesize", self.base))
            .json(req)
            .send()
            .await
            .map_err(FoniError::request)?;
        check_status(&resp)?;
        let bytes = resp.bytes().await.map_err(FoniError::request)?;
        Ok(WavData(bytes.to_vec()))
    }

    /// POST /convert — WAV → `ContentVec` → RMVPE → Generator → WAV (RVC only).
    pub async fn convert(&self, req: &ConvertRequest) -> Result<WavData> {
        let resp = self
            .http
            .post(format!("{}/convert", self.base))
            .json(req)
            .send()
            .await
            .map_err(FoniError::request)?;
        check_status(&resp)?;
        let bytes = resp.bytes().await.map_err(FoniError::request)?;
        Ok(WavData(bytes.to_vec()))
    }

    /// POST /process — apply DSP chain to existing WAV.
    pub async fn process(&self, wav: &WavData, opts: WireOpts) -> Result<WavData> {
        let body = serde_json::json!({
            "audio_data": wav.to_base64(),
            "opts": opts,
        });
        let resp = self
            .http
            .post(format!("{}/process", self.base))
            .json(&body)
            .send()
            .await
            .map_err(FoniError::request)?;
        check_status(&resp)?;
        let r: ProcessResponse = resp.json().await.map_err(FoniError::decode)?;
        WavData::from_base64(&r.audio_data)
    }

    /// POST /breath — generate breath noise WAV.
    pub async fn breath(&self, duration_ms: u32) -> Result<WavData> {
        let body = serde_json::json!({ "duration_ms": duration_ms });
        let resp = self
            .http
            .post(format!("{}/breath", self.base))
            .json(&body)
            .send()
            .await
            .map_err(FoniError::request)?;
        check_status(&resp)?;
        let r: BreathResponse = resp.json().await.map_err(FoniError::decode)?;
        WavData::from_base64(&r.audio_data)
    }

    // ── Analysis ──────────────────────────────────────────────────────────────

    /// POST /analyse — acoustic metrics; optional gap comparison against reference.
    pub async fn analyse(
        &self,
        wav: &WavData,
        reference: Option<&WavData>,
        reference_label: Option<&str>,
    ) -> Result<AnalyseResponse> {
        let body = serde_json::json!({
            "audio_data":      wav.to_base64(),
            "reference_data":  reference.map(types::WavData::to_base64),
            "reference_label": reference_label,
        });
        let resp = self
            .http
            .post(format!("{}/analyse", self.base))
            .json(&body)
            .send()
            .await
            .map_err(FoniError::request)?;
        check_status(&resp)?;
        resp.json::<AnalyseResponse>()
            .await
            .map_err(FoniError::decode)
    }

    // ── Model management ──────────────────────────────────────────────────────

    /// GET /models — list all known models and which are ONNX-ready.
    pub async fn models(&self) -> Result<ModelsResponse> {
        let resp = self
            .http
            .get(format!("{}/models", self.base))
            .send()
            .await
            .map_err(FoniError::request)?;
        check_status(&resp)?;
        resp.json::<ModelsResponse>()
            .await
            .map_err(FoniError::decode)
    }

    /// POST /models/:name — load (or pre-warm) a model into the session pool.
    pub async fn load_model(&self, name: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/models/{}", self.base, urlencoded(name)))
            .send()
            .await
            .map_err(FoniError::request)?;
        check_status(&resp)?;
        Ok(())
    }

    // ── RVC params ────────────────────────────────────────────────────────────

    /// GET /params — current RVC inference parameters.
    pub async fn params(&self) -> Result<RvcParams> {
        let resp = self
            .http
            .get(format!("{}/params", self.base))
            .send()
            .await
            .map_err(FoniError::request)?;
        check_status(&resp)?;
        resp.json::<RvcParams>().await.map_err(FoniError::decode)
    }

    /// POST /params — patch RVC parameters (partial update).
    pub async fn set_params(&self, patch: &serde_json::Value) -> Result<RvcParams> {
        let resp = self
            .http
            .post(format!("{}/params", self.base))
            .json(patch)
            .send()
            .await
            .map_err(FoniError::request)?;
        check_status(&resp)?;
        resp.json::<RvcParams>().await.map_err(FoniError::decode)
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn check_status(resp: &reqwest::Response) -> Result<()> {
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    Err(FoniError::Http(status.as_u16(), status.to_string()))
}

fn urlencoded(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                vec![c]
            } else {
                format!("%{:02X}", c as u32).chars().collect()
            }
        })
        .collect()
}

impl WavData {
    /// Decode a base64 `audio_data` string from a JSON response.
    pub fn from_base64(s: &str) -> Result<Self> {
        B64.decode(s)
            .map(WavData)
            .map_err(|e| FoniError::Decode(e.to_string()))
    }

    /// Encode for use as `audio_data` in a JSON request body.
    pub fn to_base64(&self) -> String {
        B64.encode(&self.0)
    }

    /// Raw bytes (e.g. for writing to disk or passing to /process).
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consume into raw bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
