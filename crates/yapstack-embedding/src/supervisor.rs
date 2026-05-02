use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::client::{EmbeddingClient, ModelInfo};
use crate::error::EmbeddingError;

type Result<T> = std::result::Result<T, EmbeddingError>;

/// Backoff schedule between respawn attempts when the sidecar dies.
/// Caps at 30 s so the supervisor doesn't spin during a sustained outage.
const BACKOFF_SCHEDULE_MS: &[u64] = &[1_000, 5_000, 15_000, 30_000];

/// Supervised embedding client.
///
/// The supervisor owns the active `EmbeddingClient` behind an
/// `RwLock<Arc<...>>`. Read paths (`embed`, `embed_query`, `embed_batch`)
/// take the read lock for the duration of an `Arc::clone`, then drop the
/// lock before calling into the client — so multiple in-flight embeds
/// run concurrently against the sidecar's per-id IPC routing. The write
/// lock is only held during respawn, which swaps in a fresh `Arc`.
///
/// `respawn_lock` is a separate `Mutex<()>` that ensures only one
/// respawn attempt runs at a time even if many concurrent calls observe
/// a dead client simultaneously — without it, every failing call would
/// spawn its own respawn loop.
#[derive(Clone)]
pub struct EmbeddingSupervisor {
    inner: Arc<RwLock<Arc<EmbeddingClient>>>,
    respawn_lock: Arc<Mutex<()>>,
    sidecar_path: PathBuf,
    cache_dir: Option<PathBuf>,
}

impl EmbeddingSupervisor {
    pub async fn spawn(sidecar_path: &Path, cache_dir: Option<&Path>) -> Result<Self> {
        let client = EmbeddingClient::spawn(sidecar_path, cache_dir).await?;
        Ok(Self {
            inner: Arc::new(RwLock::new(Arc::new(client))),
            respawn_lock: Arc::new(Mutex::new(())),
            sidecar_path: sidecar_path.to_path_buf(),
            cache_dir: cache_dir.map(|p| p.to_path_buf()),
        })
    }

    /// Snapshot the current client. The lock is held only for the
    /// `Arc::clone` — the returned `Arc<EmbeddingClient>` lets the caller
    /// run its embed concurrently with other embeds and with a respawn
    /// in progress.
    async fn current(&self) -> Arc<EmbeddingClient> {
        Arc::clone(&*self.inner.read().await)
    }

    pub async fn embed(&self, text: String) -> Result<Vec<f32>> {
        let client = self.current().await;
        let result = client.embed(text).await;
        if result.is_err() && !client.is_alive() {
            self.maybe_respawn(&client).await;
        }
        result
    }

    pub async fn embed_query(&self, text: String) -> Result<Vec<f32>> {
        self.embed(text).await
    }

    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let client = self.current().await;
        let result = client.embed_batch(texts).await;
        if result.is_err() && !client.is_alive() {
            self.maybe_respawn(&client).await;
        }
        result
    }

    pub async fn model_info(&self) -> Option<ModelInfo> {
        self.current().await.model_info()
    }

    pub async fn is_alive(&self) -> bool {
        self.current().await.is_alive()
    }

    /// Respawn iff `failed` is the same dead client we currently hold —
    /// otherwise another caller already swapped in a fresh one and we
    /// should not race them. The respawn_lock serializes the work so
    /// only one backoff loop runs across all concurrent failures.
    async fn maybe_respawn(&self, failed: &Arc<EmbeddingClient>) {
        let _guard = self.respawn_lock.lock().await;
        // Re-check after acquiring the respawn lock: another concurrent
        // failure may have already swapped in a healthy client while we
        // were waiting.
        {
            let current = self.inner.read().await;
            if !Arc::ptr_eq(&*current, failed) || current.is_alive() {
                return;
            }
        }
        for &delay_ms in BACKOFF_SCHEDULE_MS {
            warn!("embedding sidecar dead; respawning after {} ms", delay_ms);
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            match EmbeddingClient::spawn(&self.sidecar_path, self.cache_dir.as_deref()).await {
                Ok(new_client) => {
                    info!("embedding sidecar respawned");
                    let mut w = self.inner.write().await;
                    *w = Arc::new(new_client);
                    return;
                }
                Err(e) => warn!("respawn attempt failed: {}", e),
            }
        }
        warn!("embedding sidecar respawn gave up after backoff schedule");
    }

    pub async fn shutdown(&self) {
        let client = self.current().await;
        client.shutdown().await;
    }
}
