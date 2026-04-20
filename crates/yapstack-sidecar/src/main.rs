//! Yapstack transcription sidecar.
//!
//! A standalone binary that owns the heavy ML runtime (whisper-rs, and
//! later parakeet-rs/ort) and communicates with the main Tauri process
//! over JSON-line IPC on stdin/stdout. Logs to stderr.
//!
//! The sidecar is spawned with `--engine whisper|parakeet` (default
//! `whisper`) and instantiates the matching backend from `engines::`.
//! The IPC dispatch loop is engine-agnostic.

use std::path::PathBuf;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{error, info};
use yapstack_common::types::{EngineKind, SidecarRequest, SidecarResponse};

#[cfg(any(feature = "whisper", feature = "parakeet"))]
use crate::engines::{TranscribeOpts, TranscriptionBackend, TranscriptionOutput};

mod engines;

#[derive(Debug)]
struct CliArgs {
    engine: EngineKind,
    initial_model_path: Option<PathBuf>,
    /// Whisper-only Silero VAD model.
    vad_model_path: Option<PathBuf>,
    /// Parakeet-only Sortformer diarization model. Optional — when omitted,
    /// per-request `diarization: true` is silently treated as a no-op.
    sortformer_model_path: Option<PathBuf>,
    /// Parakeet-only persistent CoreML model cache directory. Avoids the
    /// ~5 s recompile cost on every sidecar spawn.
    coreml_cache_dir: Option<PathBuf>,
}

fn parse_args() -> CliArgs {
    let args: Vec<String> = std::env::args().collect();
    let mut engine = EngineKind::Whisper; // default preserves prior CLI behavior
    let mut initial_model_path: Option<PathBuf> = None;
    let mut vad_model_path: Option<PathBuf> = None;
    let mut sortformer_model_path: Option<PathBuf> = None;
    let mut coreml_cache_dir: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--engine" if i + 1 < args.len() => {
                engine = match args[i + 1].as_str() {
                    "whisper" => EngineKind::Whisper,
                    "parakeet" => EngineKind::Parakeet,
                    other => {
                        eprintln!("unknown --engine value: {other}; defaulting to whisper");
                        EngineKind::Whisper
                    }
                };
                i += 2;
            }
            "--model" if i + 1 < args.len() => {
                initial_model_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--vad-model" if i + 1 < args.len() => {
                vad_model_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--sortformer-model" if i + 1 < args.len() => {
                sortformer_model_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--coreml-cache-dir" if i + 1 < args.len() => {
                coreml_cache_dir = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    CliArgs {
        engine,
        initial_model_path,
        vad_model_path,
        sortformer_model_path,
        coreml_cache_dir,
    }
}

/// Construct the backend chosen by `--engine`. Returns `None` when the
/// requested engine is not compiled in (so the dispatcher can return a
/// clear error to every request).
#[cfg(any(feature = "whisper", feature = "parakeet"))]
fn build_backend(
    engine: EngineKind,
    vad_model_path: Option<PathBuf>,
    sortformer_model_path: Option<PathBuf>,
    coreml_cache_dir: Option<PathBuf>,
) -> Option<Box<dyn TranscriptionBackend>> {
    match engine {
        EngineKind::Whisper => {
            // Whisper ignores Sortformer + CoreML cache; they're parakeet-only.
            let _ = sortformer_model_path;
            let _ = coreml_cache_dir;
            #[cfg(feature = "whisper")]
            {
                Some(Box::new(engines::whisper::WhisperBackend::new(
                    vad_model_path,
                )))
            }
            #[cfg(not(feature = "whisper"))]
            {
                let _ = vad_model_path;
                None
            }
        }
        EngineKind::Parakeet => {
            // Parakeet does its own VAD-style boundary detection; the
            // Silero VAD path is whisper-only.
            let _ = vad_model_path;
            #[cfg(feature = "parakeet")]
            {
                Some(Box::new(engines::parakeet::ParakeetBackend::new(
                    sortformer_model_path,
                    coreml_cache_dir,
                )))
            }
            #[cfg(not(feature = "parakeet"))]
            {
                let _ = sortformer_model_path;
                let _ = coreml_cache_dir;
                None
            }
        }
    }
}

async fn send_response(response: &SidecarResponse) -> Result<(), Box<dyn std::error::Error>> {
    let mut json = serde_json::to_string(response)?;
    json.push('\n');
    let mut stdout = tokio::io::stdout();
    stdout.write_all(json.as_bytes()).await?;
    stdout.flush().await?;
    Ok(())
}

async fn send_error(id: u64, message: String) {
    let response = SidecarResponse::Error { id, message };
    if let Err(e) = send_response(&response).await {
        error!("failed to send error response: {}", e);
    }
}

#[tokio::main]
async fn main() {
    // Sidecar logs default to DEBUG for our own crate (so per-chunk timing
    // is visible during `pnpm tauri dev`) and INFO for everything else;
    // RUST_LOG overrides as usual.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,yapstack_sidecar=debug"));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_env_filter(filter)
        .init();

    let cli = parse_args();
    info!("sidecar starting: engine={}", cli.engine.as_str());

    if let Some(ref vad_path) = cli.vad_model_path {
        info!("VAD model path: {}", vad_path.display());
    }
    if let Some(ref s_path) = cli.sortformer_model_path {
        info!("Sortformer model path: {}", s_path.display());
    }
    if let Some(ref c_path) = cli.coreml_cache_dir {
        info!("CoreML cache dir: {}", c_path.display());
    }

    #[cfg(any(feature = "whisper", feature = "parakeet"))]
    let mut backend: Option<Box<dyn TranscriptionBackend>> = build_backend(
        cli.engine,
        cli.vad_model_path.clone(),
        cli.sortformer_model_path.clone(),
        cli.coreml_cache_dir.clone(),
    );

    #[cfg(any(feature = "whisper", feature = "parakeet"))]
    if let (Some(b), Some(model_path)) = (backend.as_mut(), cli.initial_model_path.as_ref()) {
        info!("loading initial model: {}", model_path.display());
        match b.load_model(model_path) {
            Ok(()) => info!("model loaded successfully"),
            Err(e) => error!("failed to load initial model: {}", e),
        }
    }

    #[cfg(not(any(feature = "whisper", feature = "parakeet")))]
    if cli.initial_model_path.is_some() {
        error!("no transcription engine compiled in; cannot load model");
    }

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

        let request: SidecarRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                error!("failed to parse request: {}: {}", e, line);
                continue;
            }
        };

        match request {
            SidecarRequest::Shutdown => {
                info!("shutdown requested");
                break;
            }

            SidecarRequest::LoadModel { id, model_path } => {
                info!("loading model: {}", model_path.display());

                #[cfg(any(feature = "whisper", feature = "parakeet"))]
                {
                    match backend.as_mut() {
                        Some(b) => match b.load_model(&model_path) {
                            Ok(()) => {
                                let response = SidecarResponse::ModelLoaded { id };
                                if let Err(e) = send_response(&response).await {
                                    error!("failed to send response: {}", e);
                                }
                            }
                            Err(e) => {
                                send_error(id, format!("failed to load model: {e}")).await;
                            }
                        },
                        None => {
                            send_error(
                                id,
                                format!(
                                    "engine '{}' not compiled in this build",
                                    cli.engine.as_str()
                                ),
                            )
                            .await;
                        }
                    }
                }

                #[cfg(not(any(feature = "whisper", feature = "parakeet")))]
                {
                    let _ = model_path;
                    send_error(id, "no transcription engine compiled in".to_string()).await;
                }
            }

            SidecarRequest::Transcribe {
                id,
                audio_path,
                language,
                initial_prompt,
                single_segment,
                diarization,
            } => {
                info!("transcribing: {}", audio_path.display());

                #[cfg(any(feature = "whisper", feature = "parakeet"))]
                {
                    let opts = TranscribeOpts {
                        language: language.as_deref(),
                        initial_prompt: initial_prompt.as_deref(),
                        single_segment,
                        diarization,
                    };
                    match backend.as_mut() {
                        Some(b) => match b.transcribe(&audio_path, opts) {
                            Ok(TranscriptionOutput {
                                text,
                                segments,
                                duration_ms,
                            }) => {
                                let response = SidecarResponse::Transcription {
                                    id,
                                    text,
                                    segments,
                                    duration_ms,
                                };
                                if let Err(e) = send_response(&response).await {
                                    error!("failed to send response: {}", e);
                                }
                            }
                            Err(e) => send_error(id, e).await,
                        },
                        None => {
                            send_error(
                                id,
                                format!(
                                    "engine '{}' not compiled in this build",
                                    cli.engine.as_str()
                                ),
                            )
                            .await;
                        }
                    }
                }

                #[cfg(not(any(feature = "whisper", feature = "parakeet")))]
                {
                    let _ = (
                        audio_path,
                        language,
                        initial_prompt,
                        single_segment,
                        diarization,
                    );
                    send_error(id, "no transcription engine compiled in".to_string()).await;
                }
            }
        }
    }

    info!("sidecar exiting");
}
