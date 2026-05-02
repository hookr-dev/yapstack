//! Yapstack embedding sidecar.
//!
//! Standalone binary that owns `fastembed-rs` (BGE-small-en-v1.5) and
//! communicates with the main Tauri process over JSON-line IPC on
//! stdin/stdout. Logs to stderr.
//!
//! Single engine, single model. Crash isolation from the transcription
//! sidecar is the whole point of running this as a separate process.

use std::path::PathBuf;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use yapstack_common::embedding::{EmbeddingRequest, EmbeddingResponse};

/// Stable identifier for the model. Recorded per embedding row so future
/// re-embed migrations can filter on it.
const MODEL_NAME: &str = "bge-small-en-v1.5";
/// fastembed-rs model release. Bump when switching to a quantized variant
/// or a different fastembed-rs version that changes weight bytes.
const MODEL_VERSION: &str = "1.5.0";
const MODEL_DIMENSIONS: u32 = 384;

#[derive(Debug)]
struct CliArgs {
    /// Directory where fastembed caches model weights. Mirrors the
    /// transcription sidecar's `--model` ergonomics.
    cache_dir: Option<PathBuf>,
}

fn parse_args() -> CliArgs {
    let args: Vec<String> = std::env::args().collect();
    let mut cache_dir: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--cache-dir" if i + 1 < args.len() => {
                cache_dir = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            _ => i += 1,
        }
    }
    CliArgs { cache_dir }
}

fn build_model(cache_dir: Option<PathBuf>) -> Result<TextEmbedding, fastembed::Error> {
    let mut opts = InitOptions::new(EmbeddingModel::BGESmallENV15);
    if let Some(dir) = cache_dir {
        opts = opts.with_cache_dir(dir);
    }
    TextEmbedding::try_new(opts)
}

async fn write_line(line: String) -> std::io::Result<()> {
    let mut stdout = tokio::io::stdout();
    stdout.write_all(line.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await
}

async fn send(response: EmbeddingResponse) {
    match serde_json::to_string(&response) {
        Ok(json) => {
            if let Err(e) = write_line(json).await {
                error!("failed to write response: {}", e);
            }
        }
        Err(e) => error!("failed to serialize response: {}", e),
    }
}

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("info,yapstack_embedding_sidecar=debug")
    });
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_ansi(false)
        .with_env_filter(filter)
        .init();

    let cli = parse_args();
    info!(
        "embedding sidecar starting: model={} version={}",
        MODEL_NAME, MODEL_VERSION
    );
    if let Some(ref d) = cli.cache_dir {
        info!("cache dir: {}", d.display());
    }

    // Load the model up front. If this fails, exit non-zero so the parent
    // observes a dead child instead of a process that rejects every embed.
    let model = match build_model(cli.cache_dir.clone()) {
        Ok(m) => m,
        Err(e) => {
            error!("failed to load embedding model: {}; exiting", e);
            std::process::exit(1);
        }
    };
    info!("model loaded successfully");

    // Announce readiness so the client crate can confirm liveness before
    // issuing the first request.
    send(EmbeddingResponse::Ready {
        name: MODEL_NAME.to_string(),
        version: MODEL_VERSION.to_string(),
        dimensions: MODEL_DIMENSIONS,
    })
    .await;

    // Run inference on a blocking thread — fastembed's `embed` is sync and
    // CPU-bound; we don't want it to stall the IPC reader. Use an mpsc
    // channel so the IPC loop can serialize work onto the worker.
    let (tx, mut rx) = mpsc::channel::<EmbeddingRequest>(64);
    let worker = tokio::task::spawn_blocking(move || {
        let mut model = model; // owned by the worker
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                error!("worker runtime init failed: {}", e);
                return;
            }
        };
        rt.block_on(async move {
            while let Some(req) = rx.recv().await {
                match req {
                    EmbeddingRequest::Embed { id, text } => {
                        debug!("embed id={} len={}", id, text.len());
                        match model.embed(vec![text], None) {
                            Ok(mut vectors) => {
                                if let Some(vector) = vectors.pop() {
                                    send(EmbeddingResponse::Embedded { id, vector }).await;
                                } else {
                                    send(EmbeddingResponse::Error {
                                        id,
                                        message: "empty embedding result".to_string(),
                                    })
                                    .await;
                                }
                            }
                            Err(e) => {
                                send(EmbeddingResponse::Error {
                                    id,
                                    message: format!("embed failed: {e}"),
                                })
                                .await;
                            }
                        }
                    }
                    EmbeddingRequest::EmbedBatch { id, texts } => {
                        debug!("embed_batch id={} count={}", id, texts.len());
                        match model.embed(texts, None) {
                            Ok(vectors) => {
                                send(EmbeddingResponse::EmbeddedBatch { id, vectors }).await;
                            }
                            Err(e) => {
                                send(EmbeddingResponse::Error {
                                    id,
                                    message: format!("embed_batch failed: {e}"),
                                })
                                .await;
                            }
                        }
                    }
                    EmbeddingRequest::ModelInfo { id } => {
                        send(EmbeddingResponse::ModelInfo {
                            id,
                            name: MODEL_NAME.to_string(),
                            version: MODEL_VERSION.to_string(),
                            dimensions: MODEL_DIMENSIONS,
                        })
                        .await;
                    }
                    EmbeddingRequest::Shutdown => {
                        info!("worker received shutdown");
                        break;
                    }
                }
            }
        });
    });

    info!("sidecar ready, reading from stdin");
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                info!("stdin closed, shutting down");
                break;
            }
            Err(e) => {
                error!("error reading stdin: {}", e);
                break;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: EmbeddingRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                warn!("failed to parse request: {}: {}", e, line);
                continue;
            }
        };

        let is_shutdown = matches!(request, EmbeddingRequest::Shutdown);
        if let Err(e) = tx.send(request).await {
            error!("worker channel closed: {}", e);
            break;
        }
        if is_shutdown {
            break;
        }
    }

    drop(tx);
    let _ = worker.await;
    info!("sidecar exiting");
}
