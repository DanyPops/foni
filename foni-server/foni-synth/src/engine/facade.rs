use lru::LruCache;
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::Mutex;

const CACHE_MAX_ENTRIES: usize = 256;

pub struct AudioCache {
    inner: Mutex<LruCache<String, Vec<u8>>>,
    total_bytes: Mutex<usize>,
}

impl Default for AudioCache {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(CACHE_MAX_ENTRIES).expect("infallible"),
            )),
            total_bytes: Mutex::new(0),
        }
    }

    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner.lock().await.get(key).cloned()
    }

    pub async fn put(&self, key: String, data: Vec<u8>) {
        let len = data.len();
        let mut cache = self.inner.lock().await;
        if let Some(old) = cache.put(key, data) {
            let mut total = self.total_bytes.lock().await;
            *total = total.saturating_sub(old.len());
        }
        *self.total_bytes.lock().await += len;
    }

    pub async fn clear(&self) {
        self.inner.lock().await.clear();
        *self.total_bytes.lock().await = 0;
    }

    pub async fn stats(&self) -> (usize, usize) {
        let cache = self.inner.lock().await;
        let total = *self.total_bytes.lock().await;
        (cache.len(), total)
    }
}

pub fn cache_key(text: &str, model: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{model}|{text}"));
    format!("{:x}", hasher.finalize())
}

/// Generation-tagged audio queue.
///
/// Each chunk is tagged with the generation in which it was enqueued.
/// Calling `clear()` atomically bumps the generation; the player task
/// skips any chunk whose tag is older than the current generation,
/// draining pending audio instantly without blocking or sleeping.
pub struct PlayQueue {
    tx: tokio::sync::mpsc::Sender<(u64, Vec<u8>)>,
    generation: Arc<std::sync::atomic::AtomicU64>,
}

impl PlayQueue {
    pub fn new() -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(u64, Vec<u8>)>(32);
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let gen_reader = generation.clone();

        let handle = tokio::spawn(async move {
            while let Some((item_gen, wav)) = rx.recv().await {
                // Skip chunks from a cleared (old) generation.
                if item_gen != gen_reader.load(std::sync::atomic::Ordering::Acquire) {
                    continue;
                }
                if let Err(e) = super::player::play_wav_async(wav).await {
                    tracing::warn!("playback failed: {e}");
                }
            }
        });
        (Self { tx, generation }, handle)
    }

    pub async fn enqueue(&self, wav: Vec<u8>) {
        let gen = self.generation.load(std::sync::atomic::Ordering::Relaxed);
        let _ = self.tx.send((gen, wav)).await;
    }

    /// Immediately discard all queued audio by bumping the generation counter.
    ///
    /// Chunks already in the channel are received by the player task but
    /// skipped because their generation tag is stale. The next `enqueue`
    /// call uses the new generation and plays normally.
    pub fn clear(&self) {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        tracing::debug!("play_queue: cleared (generation bumped)");
    }
}

pub type SharedCache = Arc<AudioCache>;

pub fn new_shared_cache() -> SharedCache {
    Arc::new(AudioCache::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cache_put_and_get() {
        let cache = AudioCache::new();
        cache.put("k1".into(), vec![1, 2, 3]).await;
        assert_eq!(cache.get("k1").await, Some(vec![1, 2, 3]));
    }

    #[tokio::test]
    async fn cache_miss_returns_none() {
        let cache = AudioCache::new();
        assert_eq!(cache.get("nope").await, None);
    }

    #[tokio::test]
    async fn cache_clear_empties() {
        let cache = AudioCache::new();
        cache.put("k1".into(), vec![1]).await;
        cache.clear().await;
        assert_eq!(cache.get("k1").await, None);
        let (len, bytes) = cache.stats().await;
        assert_eq!(len, 0);
        assert_eq!(bytes, 0);
    }

    #[tokio::test]
    async fn cache_stats_tracks_size() {
        let cache = AudioCache::new();
        cache.put("a".into(), vec![0; 100]).await;
        cache.put("b".into(), vec![0; 200]).await;
        let (entries, bytes) = cache.stats().await;
        assert_eq!(entries, 2);
        assert_eq!(bytes, 300);
    }

    #[test]
    fn cache_key_is_deterministic() {
        let k1 = cache_key("hello", "sidorovich");
        let k2 = cache_key("hello", "sidorovich");
        assert_eq!(k1, k2);
    }

    #[test]
    fn cache_key_differs_per_model() {
        let k1 = cache_key("hello", "sidorovich");
        let k2 = cache_key("hello", "other");
        assert_ne!(k1, k2);
    }
}
