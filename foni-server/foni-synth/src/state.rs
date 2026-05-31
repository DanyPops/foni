use std::{path::PathBuf, sync::Arc};

use lru::LruCache;
use tokio::sync::{Mutex, RwLock};

use crate::config::ServerConfig;

// ─── ONNX session pool ────────────────────────────────────────────────────────

/// Loaded ONNX sessions held in memory for the lifetime of the server.
/// All three models are ~800 MB combined — we load once, reuse forever.
pub struct OnnxPool {
    pub contentvec: ort::session::Session,
    pub rmvpe: ort::session::Session,
    pub generator: ort::session::Session,
    /// Model name the generator belongs to (e.g. "bandit").
    pub model_name: String,
}

/// Lazy-initialized pool, behind a Mutex so callers serialize inference.
/// `None` until the first `/convert` call or explicit `POST /models/:name`.
pub type SessionPool = Arc<Mutex<Option<OnnxPool>>>;

// ─── WAV cache ────────────────────────────────────────────────────────────────

/// In-memory LRU cache: SHA-256(text + model + opts) → raw WAV bytes.
/// Capacity chosen so the cache stays under ~200 MB (≈ 500 × 400 KB avg).
pub const WAV_CACHE_CAPACITY: usize = 500;

pub type WavCache = Arc<Mutex<LruCache<[u8; 32], Vec<u8>>>>;

// ─── RVC params ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RvcParams {
    pub f0up_key: i32,
    pub index_rate: f32,
    pub filter_radius: u32,
    pub rms_mix_rate: f32,
    pub protect: f32,
    pub f0method: String,
}

impl Default for RvcParams {
    fn default() -> Self {
        RvcParams {
            f0up_key: -2,
            index_rate: 0.77,
            filter_radius: 5,
            rms_mix_rate: 0.45,
            protect: 0.33,
            f0method: "rmvpe".to_string(),
        }
    }
}

// ─── App state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub current_model: RwLock<Option<String>>,
    pub params: RwLock<RvcParams>,
    pub models_dir: PathBuf,
    pub sessions: SessionPool,
    pub wav_cache: WavCache,
}

impl AppState {
    pub fn from_config(cfg: ServerConfig) -> Self {
        AppState(Arc::new(Inner {
            current_model: RwLock::new(cfg.initial_model),
            params: RwLock::new(cfg.params),
            models_dir: cfg.models_dir,
            sessions: Arc::new(Mutex::new(None)),
            wav_cache: Arc::new(Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(WAV_CACHE_CAPACITY).unwrap(),
            ))),
        }))
    }

    /// How many entries are in the WAV cache right now.
    pub async fn cache_len(&self) -> usize {
        self.0.wav_cache.lock().await.len()
    }
}
