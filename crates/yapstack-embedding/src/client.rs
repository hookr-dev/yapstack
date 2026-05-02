use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex as TokioMutex};
use tokio::time::timeout;
use tracing::{debug, info, warn};
use yapstack_common::embedding::{EmbeddingRequest, EmbeddingResponse};

use crate::error::EmbeddingError;

type Result<T> = std::result::Result<T, EmbeddingError>;
type ResponseWaiter = oneshot::Sender<EmbeddingResponse>;
type PendingMap = Arc<StdMutex<HashMap<u64, ResponseWaiter>>>;

/// Single-call timeout. fastembed-rs returns in ~10–30 ms for short text on
/// CPU; 30 s is a safety cap that catches a wedged worker without inducing
/// false errors during the cold-start of the first request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: String,
    pub version: String,
    pub dimensions: u32,
}

struct SidecarParts {
    stdin: ChildStdin,
    pending: PendingMap,
    child: Child,
    reader_alive: Arc<AtomicBool>,
    last_model_info: Arc<StdMutex<Option<ModelInfo>>>,
    /// Fires once when the sidecar emits its `Ready` banner. `spawn`
    /// awaits this before returning so a sidecar that dies during model
    /// download surfaces as a spawn error rather than a silently-installed
    /// dead client.
    ready_rx: oneshot::Receiver<()>,
}

/// Client for `yapstack-embedding-sidecar`. Supports concurrent in-flight
/// requests via per-id oneshot waiters. All public methods take `&self`
/// — every piece of mutable state is interior-mutable so the client can
/// be held behind an `Arc` and embedded into a supervisor that allows
/// concurrent reads.
///
/// Respawning lives on the supervisor, which discards a dead `Arc` and
/// installs a fresh one — this client owns no respawn state of its own.
pub struct EmbeddingClient {
    stdin: TokioMutex<ChildStdin>,
    pending: PendingMap,
    next_id: AtomicU64,
    child: StdMutex<Child>,
    reader_alive: Arc<AtomicBool>,
    last_model_info: Arc<StdMutex<Option<ModelInfo>>>,
}

/// Upper bound on how long `spawn` waits for the sidecar to emit
/// `Ready`. Covers cold-start: model download (~67 MB on slow links)
/// and ONNX runtime init.
const READY_TIMEOUT: Duration = Duration::from_secs(120);

impl EmbeddingClient {
    /// Spawn the sidecar binary and wait for its `Ready` banner.
    /// Returns `Err(SidecarDead)` if the child exits before Ready or if
    /// the timeout elapses — callers must treat a successful return as
    /// "the model is loaded and the sidecar is serving requests."
    pub async fn spawn(sidecar_path: &Path, cache_dir: Option<&Path>) -> Result<Self> {
        let parts = Self::spawn_sidecar(sidecar_path, cache_dir).await?;
        let SidecarParts {
            stdin,
            pending,
            child,
            reader_alive,
            last_model_info,
            ready_rx,
        } = parts;

        let client = Self {
            stdin: TokioMutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            child: StdMutex::new(child),
            reader_alive: Arc::clone(&reader_alive),
            last_model_info,
        };

        // Block until the sidecar reports Ready. If the reader task
        // exits first (child died during model load),
        // `reader_alive` flips to false; we poll both signals via a
        // tokio::select! and fail fast on either.
        let ready_alive = Arc::clone(&reader_alive);
        let wait = async {
            tokio::select! {
                ready = ready_rx => {
                    match ready {
                        Ok(()) => Ok(()),
                        Err(_) => Err(EmbeddingError::SidecarDead),
                    }
                }
                _ = async {
                    // Poll liveness — the reader sets this false on EOF.
                    while ready_alive.load(Ordering::Acquire) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                } => Err(EmbeddingError::SidecarDead),
            }
        };
        match tokio::time::timeout(READY_TIMEOUT, wait).await {
            Ok(Ok(())) => Ok(client),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(EmbeddingError::Timeout),
        }
    }

    async fn spawn_sidecar(sidecar_path: &Path, cache_dir: Option<&Path>) -> Result<SidecarParts> {
        info!(
            "spawning embedding sidecar: {} cache_dir={:?}",
            sidecar_path.display(),
            cache_dir.as_ref().map(|p| p.display().to_string())
        );

        let mut cmd = tokio::process::Command::new(sidecar_path);
        if let Some(dir) = cache_dir {
            cmd.arg("--cache-dir").arg(dir);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            EmbeddingError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "child stdin missing",
            ))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            EmbeddingError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "child stdout missing",
            ))
        })?;

        let pending: PendingMap = Arc::new(StdMutex::new(HashMap::new()));
        let reader_alive = Arc::new(AtomicBool::new(true));
        let last_model_info: Arc<StdMutex<Option<ModelInfo>>> = Arc::new(StdMutex::new(None));
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        // Stash the sender behind a Mutex<Option<...>> so the reader can
        // .take() it on first Ready. Subsequent Readys (which shouldn't
        // happen, but defensively) are silently dropped.
        let ready_tx = Arc::new(StdMutex::new(Some(ready_tx)));

        // Forward stderr to tracing so sidecar logs land in the parent's log surface.
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    debug!(target: "embedding_sidecar", "{}", line);
                }
            });
        }

        // Reader task — routes responses to per-id oneshot waiters.
        let reader_pending = Arc::clone(&pending);
        let reader_alive_inner = Arc::clone(&reader_alive);
        let reader_model_info = Arc::clone(&last_model_info);
        let reader_ready_tx = Arc::clone(&ready_tx);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let response: EmbeddingResponse = match serde_json::from_str(&line) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!("failed to parse sidecar line: {}: {}", e, line);
                        continue;
                    }
                };
                match response {
                    EmbeddingResponse::Ready {
                        ref name,
                        ref version,
                        dimensions,
                    } => {
                        info!(
                            "sidecar ready: model={} version={} dim={}",
                            name, version, dimensions
                        );
                        if let Ok(mut guard) = reader_model_info.lock() {
                            *guard = Some(ModelInfo {
                                name: name.clone(),
                                version: version.clone(),
                                dimensions,
                            });
                        }
                        if let Ok(mut tx_guard) = reader_ready_tx.lock() {
                            if let Some(tx) = tx_guard.take() {
                                let _ = tx.send(());
                            }
                        }
                    }
                    EmbeddingResponse::Embedded { id, .. }
                    | EmbeddingResponse::EmbeddedBatch { id, .. }
                    | EmbeddingResponse::ModelInfo { id, .. }
                    | EmbeddingResponse::Error { id, .. } => {
                        let waiter = reader_pending.lock().ok().and_then(|mut m| m.remove(&id));
                        if let Some(tx) = waiter {
                            let _ = tx.send(response);
                        } else {
                            warn!("response for unknown id {}", id);
                        }
                    }
                }
            }
            reader_alive_inner.store(false, Ordering::Release);
            // Fail any in-flight waiters so callers don't hang.
            if let Ok(mut m) = reader_pending.lock() {
                m.clear();
            }
            info!("embedding sidecar reader task ended");
        });

        Ok(SidecarParts {
            stdin,
            pending,
            child,
            reader_alive,
            last_model_info,
            ready_rx,
        })
    }

    pub fn is_alive(&self) -> bool {
        self.reader_alive.load(Ordering::Acquire)
    }

    pub fn model_info(&self) -> Option<ModelInfo> {
        self.last_model_info.lock().ok().and_then(|g| g.clone())
    }

    async fn send_and_wait(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        if !self.is_alive() {
            return Err(EmbeddingError::SidecarDead);
        }
        let id = match request {
            EmbeddingRequest::Embed { id, .. }
            | EmbeddingRequest::EmbedBatch { id, .. }
            | EmbeddingRequest::ModelInfo { id } => id,
            EmbeddingRequest::Shutdown => unreachable!("shutdown does not wait"),
        };

        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| EmbeddingError::SidecarDead)?
            .insert(id, tx);

        let mut json = serde_json::to_string(&request)?;
        json.push('\n');
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(json.as_bytes()).await?;
            stdin.flush().await?;
        }

        match timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(EmbeddingError::ResponseDropped),
            Err(_) => {
                // Drop the now-orphan waiter to bound the pending map.
                if let Ok(mut m) = self.pending.lock() {
                    m.remove(&id);
                }
                Err(EmbeddingError::Timeout)
            }
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Embed a single text. Used by both write paths (segments / dictations
    /// / notes) and the read path. fastembed truncates inputs longer than
    /// the model context (512 tokens for BGE-small).
    pub async fn embed(&self, text: String) -> Result<Vec<f32>> {
        let id = self.next_id();
        match self
            .send_and_wait(EmbeddingRequest::Embed { id, text })
            .await?
        {
            EmbeddingResponse::Embedded { vector, .. } => Ok(vector),
            EmbeddingResponse::Error { message, .. } => Err(EmbeddingError::SidecarError(message)),
            other => Err(EmbeddingError::SidecarError(format!(
                "unexpected response: {:?}",
                std::mem::discriminant(&other)
            ))),
        }
    }

    /// Semantically same as `embed`, but kept as its own method so call
    /// sites document intent (read path vs write path).
    pub async fn embed_query(&self, text: String) -> Result<Vec<f32>> {
        self.embed(text).await
    }

    /// Embed a batch of texts in a single forward pass. Used by the
    /// backfill worker.
    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let id = self.next_id();
        match self
            .send_and_wait(EmbeddingRequest::EmbedBatch { id, texts })
            .await?
        {
            EmbeddingResponse::EmbeddedBatch { vectors, .. } => Ok(vectors),
            EmbeddingResponse::Error { message, .. } => Err(EmbeddingError::SidecarError(message)),
            other => Err(EmbeddingError::SidecarError(format!(
                "unexpected response: {:?}",
                std::mem::discriminant(&other)
            ))),
        }
    }

    /// Best-effort graceful shutdown. Takes `&self` because every mutable
    /// piece of state (`stdin`, `child`) is already interior-mutable —
    /// the `&mut` ergonomics are unnecessary and prevent us from holding
    /// the client behind an `Arc` for concurrent use.
    pub async fn shutdown(&self) {
        let json = match serde_json::to_string(&EmbeddingRequest::Shutdown) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut payload = json;
        payload.push('\n');
        {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.write_all(payload.as_bytes()).await;
            let _ = stdin.flush().await;
        }
        // Give the worker a moment to exit; then kill.
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Ok(mut child) = self.child.lock() {
            let _ = child.start_kill();
        }
    }
}
