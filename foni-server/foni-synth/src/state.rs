use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub current_model:  RwLock<Option<String>>,
    pub params:         RwLock<RvcParams>,
    pub models_dir:     std::path::PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RvcParams {
    pub f0up_key:      i32,
    pub index_rate:    f32,
    pub filter_radius: u32,
    pub rms_mix_rate:  f32,
    pub protect:       f32,
    pub f0method:      String,
}

impl Default for RvcParams {
    fn default() -> Self {
        RvcParams {
            f0up_key:      -2,
            index_rate:    0.77,
            filter_radius: 5,
            rms_mix_rate:  0.45,
            protect:       0.33,
            f0method:      "rmvpe".to_string(),
        }
    }
}

impl AppState {
    pub async fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let models_dir = std::env::var("RVC_MODELS_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("/app/rvc_models"));

        Ok(AppState(Arc::new(Inner {
            current_model: RwLock::new(None),
            params:        RwLock::new(RvcParams::default()),
            models_dir,
        })))
    }
}


