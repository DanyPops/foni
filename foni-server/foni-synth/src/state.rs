use std::{path::PathBuf, sync::Arc};

use lru::LruCache;
use tokio::sync::{Mutex, RwLock};

use crate::config::ResolvedConfig;
use crate::engine::synth_backend::{modal_backend, SharedSynth};

// ── WAV cache ─────────────────────────────────────────────────────────────────

pub const WAV_CACHE_CAPACITY: usize = 500;
pub type WavCache = Arc<Mutex<LruCache<[u8; 32], Vec<u8>>>>;

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub current_model: RwLock<Option<String>>,
    pub params: RwLock<crate::config::RvcParams>,
    pub models_dir: PathBuf,
    pub wav_cache: WavCache,
    pub dsp_enabled: std::sync::atomic::AtomicBool,
    pub controller_enabled: std::sync::atomic::AtomicBool,
    pub controller_config: RwLock<crate::quality::dsp::controller::ControllerConfig>,

    pub breaks_config: RwLock<crate::config::BreaksConfig>,
    pub dsp_defaults: RwLock<crate::config::DspDefaults>,
    pub policy_engine: RwLock<Option<std::sync::Arc<crate::quality::dsp::policy::PolicyEngine>>>,
    /// Shared TTS backend — injected at startup, never self-calls the server.
    pub synth: SharedSynth,
}

impl AppState {
    pub fn from_config(cfg: ResolvedConfig) -> Self {
        AppState(Arc::new(Inner {
            current_model: RwLock::new(cfg.initial_model),
            params: RwLock::new(cfg.params),
            models_dir: cfg.models_dir,
            wav_cache: Arc::new(Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(WAV_CACHE_CAPACITY)
                    .expect("infallible: WAV_CACHE_CAPACITY is non-zero"),
            ))),
            dsp_enabled: std::sync::atomic::AtomicBool::new(true),
            controller_enabled: std::sync::atomic::AtomicBool::new(cfg.controller.enabled),
            controller_config: RwLock::new(cfg.controller),

            breaks_config: RwLock::new(cfg.breaks),
            dsp_defaults: RwLock::new(cfg.dsp),
            policy_engine: RwLock::new(
                crate::quality::dsp::policy::find_policy_script()
                    .and_then(|p| crate::quality::dsp::policy::PolicyEngine::load(&p))
                    .map(std::sync::Arc::new),
            ),
            synth: modal_backend(),
        }))
    }

    pub async fn cache_len(&self) -> usize {
        self.0.wav_cache.lock().await.len()
    }

    /// Build with a custom synth backend — used in tests to inject a mock.
    pub fn from_config_with_synth(cfg: ResolvedConfig, synth: SharedSynth) -> Self {
        let mut state = Self::from_config(cfg);
        // Replace the default modal backend with the injected one.
        // Safety: we just created this Arc, no other references exist yet.
        Arc::get_mut(&mut state.0)
            .expect("sole owner at construction")
            .synth = synth;
        state
    }
}
