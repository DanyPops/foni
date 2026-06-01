use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};

use lru::LruCache;
use tokio::sync::{Mutex, RwLock, Semaphore, SemaphorePermit};

use crate::config::ResolvedConfig;
use crate::voice_index::VoiceIndex;

// ── ONNX session (one inference context) ─────────────────────────────────────

pub struct OnnxSession {
    pub contentvec: ort::session::Session,
    pub rmvpe: ort::session::Session,
    pub generator: ort::session::Session,
    pub model_name: String,
}

// ── Pool guard — holds a session and returns it on drop ───────────────────────

pub struct PoolGuard<'a> {
    pub session: tokio::sync::MutexGuard<'a, Option<OnnxSession>>,
    _permit: SemaphorePermit<'a>,
}

// ── Session pool ──────────────────────────────────────────────────────────────

pub struct SessionPool {
    pub slots: Vec<Arc<Mutex<Option<OnnxSession>>>>,
    sem: Arc<Semaphore>,
    pub size: usize,

    pub requests_total: AtomicU64,
    pub cache_hits: AtomicU64,
    pub active: AtomicUsize,
    pub waiting: AtomicUsize,

    /// Ring buffer of recent request durations (ms) for percentile computation.
    latencies: Mutex<std::collections::VecDeque<u64>>,
}

impl SessionPool {
    pub fn new(size: usize) -> Arc<Self> {
        let mut slots = Vec::with_capacity(size);
        for _ in 0..size {
            slots.push(Arc::new(Mutex::new(None)));
        }
        Arc::new(Self {
            slots,
            sem: Arc::new(Semaphore::new(size)),
            size,
            requests_total: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            active: AtomicUsize::new(0),
            waiting: AtomicUsize::new(0),
            latencies: Mutex::new(std::collections::VecDeque::with_capacity(1024)),
        })
    }

    /// Acquire a slot. Waits until a session is free.
    /// Returns the first unlocked slot (round-robin-ish via try_lock scan).
    pub async fn acquire(&self) -> PoolGuard<'_> {
        self.waiting.fetch_add(1, Ordering::Relaxed);
        let permit = self.sem.acquire().await.expect("semaphore closed");
        self.waiting.fetch_sub(1, Ordering::Relaxed);
        self.active.fetch_add(1, Ordering::Relaxed);
        self.requests_total.fetch_add(1, Ordering::Relaxed);

        // Scan slots for one that's unlocked — the permit guarantees one is free.
        for slot in &self.slots {
            if let Ok(guard) = slot.try_lock() {
                return PoolGuard {
                    session: guard,
                    _permit: permit,
                };
            }
        }
        // Fallback: wait on the first slot (should not happen in practice).
        PoolGuard {
            session: self.slots[0].lock().await,
            _permit: permit,
        }
    }

    pub fn release_active(&self) {
        self.active.fetch_sub(1, Ordering::Relaxed);
    }

    pub async fn record_latency(&self, ms: u64) {
        let mut q = self.latencies.lock().await;
        if q.len() >= 1024 {
            q.pop_front();
        }
        q.push_back(ms);
    }

    pub async fn percentiles(&self) -> (u64, u64, u64) {
        let q = self.latencies.lock().await;
        if q.is_empty() {
            return (0, 0, 0);
        }
        let mut v: Vec<u64> = q.iter().copied().collect();
        v.sort_unstable();
        let n = v.len();
        let p = |pct: f64| v[(pct / 100.0 * n as f64).min(n as f64 - 1.0) as usize];
        (p(50.0), p(95.0), p(99.0))
    }

    /// True if all slots have a loaded model matching `model_name`.
    pub async fn all_loaded(&self, model_name: &str) -> bool {
        for slot in &self.slots {
            let g = slot.lock().await;
            match &*g {
                None => return false,
                Some(s) if s.model_name != model_name => return false,
                _ => {}
            }
        }
        true
    }
}

pub type SharedSessionPool = Arc<SessionPool>;

// ── WAV cache ─────────────────────────────────────────────────────────────────

pub const WAV_CACHE_CAPACITY: usize = 500;
pub type WavCache = Arc<Mutex<LruCache<[u8; 32], Vec<u8>>>>;

// ── RVC params ────────────────────────────────────────────────────────────────

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

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub current_model: RwLock<Option<String>>,
    pub params: RwLock<RvcParams>,
    pub models_dir: PathBuf,
    pub sessions: SharedSessionPool,
    pub wav_cache: WavCache,
    pub voice_index: RwLock<Option<VoiceIndex>>,
    pub controller_enabled: std::sync::atomic::AtomicBool,
    pub controller_config: RwLock<crate::dsp::controller::ControllerConfig>,
}

impl AppState {
    pub fn from_config(cfg: ResolvedConfig) -> Self {
        let pool_size = std::env::var("FONI_POOL_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(4)
            .max(1);

        tracing::info!("session pool size: {pool_size}");

        AppState(Arc::new(Inner {
            current_model: RwLock::new(cfg.initial_model),
            params: RwLock::new(cfg.params),
            models_dir: cfg.models_dir,
            sessions: SessionPool::new(pool_size),
            wav_cache: Arc::new(Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(WAV_CACHE_CAPACITY).unwrap(),
            ))),
            voice_index: RwLock::new(None),
            controller_enabled: std::sync::atomic::AtomicBool::new(cfg.controller.enabled),
            controller_config: RwLock::new(cfg.controller),
        }))
    }

    pub async fn cache_len(&self) -> usize {
        self.0.wav_cache.lock().await.len()
    }
}

// Backward-compat alias — routes import OnnxPool by name.
pub use OnnxSession as OnnxPool;
