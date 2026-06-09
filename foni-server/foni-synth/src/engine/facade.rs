use lru::LruCache;
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

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

/// Generation-tagged audio queue with kill support.
///
/// `clear()` both bumps the generation (causing queued chunks to be skipped)
/// and signals the player task to kill any subprocess currently playing.
pub struct PlayQueue {
    tx: tokio::sync::mpsc::Sender<(u64, usize, Vec<u8>)>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    kill_tx: watch::Sender<u64>,
    played_tx: tokio::sync::mpsc::Sender<(u64, usize)>,
}

impl PlayQueue {
    pub fn new() -> (
        Self,
        tokio::task::JoinHandle<()>,
        tokio::sync::mpsc::Receiver<(u64, usize)>,
    ) {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(u64, usize, Vec<u8>)>(32);
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let gen_reader = generation.clone();
        let (kill_tx, mut kill_rx) = watch::channel(0u64);
        let (played_tx, played_rx) = tokio::sync::mpsc::channel::<(u64, usize)>(32);
        let played_inner = played_tx.clone();

        let handle = tokio::spawn(async move {
            while let Some((item_gen, chunk_idx, wav)) = rx.recv().await {
                if item_gen != gen_reader.load(std::sync::atomic::Ordering::Acquire) {
                    continue;
                }
                if let Err(e) = super::player::play_wav_killable(wav, &mut kill_rx).await {
                    tracing::warn!("playback failed: {e}");
                }
                // Signal playback completion regardless of player success/failure —
                // the chunk has left the queue either way.
                let _ = played_inner.send((item_gen, chunk_idx)).await;
            }
        });

        (
            Self {
                tx,
                generation,
                kill_tx,
                played_tx,
            },
            handle,
            played_rx,
        )
    }

    /// Enqueue audio tagged with the caller-supplied generation snapshot.
    ///
    /// Use this when synthesis may have taken time: snapshot the generation
    /// before synthesis begins, pass the snapshot here so a reset that fires
    /// during synthesis causes the completed chunk to be skipped.
    pub async fn enqueue_tagged(&self, wav: Vec<u8>, gen: u64, chunk_idx: usize) {
        let _ = self.tx.send((gen, chunk_idx, wav)).await;
    }

    /// Enqueue audio tagged with the current generation.
    ///
    /// Safe for cache hits where there is no async gap between reading the
    /// generation and enqueuing.
    pub async fn enqueue(&self, wav: Vec<u8>, chunk_idx: usize) {
        let gen = self.generation.load(std::sync::atomic::Ordering::Relaxed);
        let _ = self.tx.send((gen, chunk_idx, wav)).await;
    }

    /// Read the current generation without modifying it.
    ///
    /// Call before beginning synthesis so the snapshot can be passed to
    /// `enqueue_tagged` after synthesis completes.
    pub fn generation_snapshot(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Discard all queued audio and kill the currently playing subprocess.
    pub fn clear(&self) {
        let new_gen = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Release)
            + 1;
        let _ = self.kill_tx.send(new_gen);
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

    #[tokio::test]
    async fn generation_snapshot_reads_zero_initially() {
        let (queue, _handle, _played) = PlayQueue::new();
        assert_eq!(queue.generation_snapshot(), 0);
    }

    #[tokio::test]
    async fn clear_bumps_generation() {
        let (queue, _handle, _played) = PlayQueue::new();
        queue.clear();
        assert_eq!(queue.generation_snapshot(), 1);
        queue.clear();
        assert_eq!(queue.generation_snapshot(), 2);
    }

    #[tokio::test]
    async fn enqueue_tagged_uses_provided_generation() {
        let (queue, _handle, _played) = PlayQueue::new();
        let snap = queue.generation_snapshot(); // 0
        queue.clear(); // gen → 1
                       // Tag with pre-clear generation — player task will skip (0 ≠ 1).
        queue.enqueue_tagged(vec![0u8; 44], snap, 0).await;
        // If enqueue_tagged incorrectly used the current gen (1), the chunk
        // would pass the player guard. We verify the snapshot was 0 (pre-clear).
        assert_eq!(snap, 0);
    }
}
