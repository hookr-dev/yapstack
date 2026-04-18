use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::Emitter;
use tokio::sync::Mutex;
use tracing::{error, info};
use yapstack_transcription::{ModelManager, ModelSize, TranscriptionResult, WhisperClient};

use super::error::CommandError;

// --- DTOs ---

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum ModelSizeDto {
    Tiny,
    Base,
    Small,
    Medium,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ModelInfoDto {
    pub size: ModelSizeDto,
    pub downloaded: bool,
    pub path: Option<String>,
    pub display_name: String,
    pub approximate_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TranscriptionResultDto {
    pub text: String,
    pub segments: Vec<TranscriptSegmentDto>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TranscriptSegmentDto {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub confidence: f32,
}

// --- From impls ---

impl From<ModelSizeDto> for ModelSize {
    fn from(s: ModelSizeDto) -> Self {
        match s {
            ModelSizeDto::Tiny => ModelSize::Tiny,
            ModelSizeDto::Base => ModelSize::Base,
            ModelSizeDto::Small => ModelSize::Small,
            ModelSizeDto::Medium => ModelSize::Medium,
        }
    }
}

impl From<ModelSize> for ModelSizeDto {
    fn from(s: ModelSize) -> Self {
        match s {
            ModelSize::Tiny => ModelSizeDto::Tiny,
            ModelSize::Base => ModelSizeDto::Base,
            ModelSize::Small => ModelSizeDto::Small,
            ModelSize::Medium => ModelSizeDto::Medium,
        }
    }
}

impl From<yapstack_transcription::ModelInfo> for ModelInfoDto {
    fn from(m: yapstack_transcription::ModelInfo) -> Self {
        Self {
            size: m.size.into(),
            downloaded: m.downloaded,
            path: m.path.map(|p| p.to_string_lossy().into_owned()),
            display_name: m.display_name,
            approximate_size_bytes: m.approximate_size_bytes,
        }
    }
}

impl From<TranscriptionResult> for TranscriptionResultDto {
    fn from(r: TranscriptionResult) -> Self {
        Self {
            text: r.text,
            segments: r
                .segments
                .into_iter()
                .map(|s| TranscriptSegmentDto {
                    start_ms: s.start_ms,
                    end_ms: s.end_ms,
                    text: s.text,
                    confidence: s.confidence,
                })
                .collect(),
            duration_ms: r.duration_ms,
        }
    }
}

// --- State types ---

pub type ModelManagerState = Arc<Mutex<ModelManager>>;
pub type WhisperClientState = Arc<Mutex<Option<WhisperClient>>>;

// --- Commands ---

#[tauri::command]
#[specta::specta]
pub async fn get_available_models(
    state: tauri::State<'_, ModelManagerState>,
) -> Result<Vec<ModelInfoDto>, CommandError> {
    let manager = state.lock().await;
    Ok(manager
        .list_all()
        .into_iter()
        .map(ModelInfoDto::from)
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn download_model(
    state: tauri::State<'_, ModelManagerState>,
    window: tauri::Window,
    size: ModelSizeDto,
) -> Result<String, CommandError> {
    info!(size = ?size, "downloading model");
    // Clone the manager so we can release the lock before the long download.
    // ModelManager is cheap to clone (just a PathBuf).
    let manager = {
        let guard = state.lock().await;
        guard.clone()
    };
    let model_size: ModelSize = size.into();

    let path = manager
        .download(model_size, move |progress| {
            let _ = window.emit(
                "model-download-progress",
                serde_json::json!({
                    "percent": progress,
                    "size": format!("{:?}", model_size),
                }),
            );
        })
        .await
        .map_err(|e| {
            error!("model download failed: {}", e);
            CommandError::from(e)
        })?;

    info!(path = %path.display(), "model downloaded");
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_model(
    state: tauri::State<'_, ModelManagerState>,
    size: ModelSizeDto,
) -> Result<(), CommandError> {
    info!(size = ?size, "deleting model");
    // Clone the manager so we can release the lock before async filesystem I/O.
    let manager = {
        let guard = state.lock().await;
        guard.clone()
    };
    manager.delete(size.into()).await.map_err(|e| {
        error!("model deletion failed: {}", e);
        CommandError::from(e)
    })
}

#[tauri::command]
#[specta::specta]
pub async fn transcribe_audio(
    state: tauri::State<'_, WhisperClientState>,
    audio_path: String,
    language: Option<String>,
    initial_prompt: Option<String>,
) -> Result<TranscriptionResultDto, CommandError> {
    info!(path = %audio_path, language = ?language, "transcribing audio");
    let mut client_guard = state.lock().await;
    let client = client_guard.as_mut().ok_or(CommandError::NotInitialized {
        message: "transcription engine not initialized".into(),
    })?;

    let result = client
        .transcribe(
            &PathBuf::from(&audio_path),
            language.as_deref(),
            initial_prompt.as_deref(),
        )
        .await
        .map_err(|e| {
            error!("transcription failed: {}", e);
            CommandError::from(e)
        })?;

    info!(
        duration_ms = result.duration_ms,
        text_len = result.text.len(),
        segments = result.segments.len(),
        "transcription complete"
    );
    Ok(TranscriptionResultDto::from(result))
}

#[tauri::command]
#[specta::specta]
pub async fn init_whisper_client(
    model_manager_state: tauri::State<'_, ModelManagerState>,
    whisper_state: tauri::State<'_, WhisperClientState>,
    size: ModelSizeDto,
) -> Result<(), CommandError> {
    info!(size = ?size, "initializing whisper client");
    let model_size: ModelSize = size.into();

    // Idempotent: if a client is already live, don't respawn. A concurrent
    // caller (Vite HMR remount, React StrictMode double-mount, rapid UI
    // clicks) that races through the check-and-spawn window would otherwise
    // leave two sidecar processes with the old one orphaned.
    {
        let client_guard = whisper_state.lock().await;
        if client_guard.is_some() {
            info!("whisper client already initialized, skipping respawn");
            return Ok(());
        }
    }

    // Extract paths under lock, then drop lock before the potentially slow spawn
    let (model_path, sidecar_path, vad_model_path) = {
        let manager = model_manager_state.lock().await;
        let mp = manager
            .model_path(model_size)
            .ok_or(CommandError::NotFound {
                message: format!("model {:?} not downloaded", model_size),
            })?;
        let sp = find_sidecar_path()?;

        // Ensure VAD model is available (auto-download if missing, only ~885KB)
        let vad_path = match manager.ensure_vad_model().await {
            Ok(path) => {
                info!(path = %path.display(), "VAD model ready");
                Some(path)
            }
            Err(e) => {
                // VAD is optional — log but don't fail whisper init
                error!("failed to ensure VAD model: {}, proceeding without VAD", e);
                None
            }
        };

        (mp, sp, vad_path)
    }; // lock dropped

    let client = WhisperClient::spawn(&sidecar_path, &model_path, vad_model_path.as_deref())
        .await
        .map_err(|e| {
            error!("failed to spawn whisper client: {}", e);
            CommandError::from(e)
        })?;

    // Re-check under the write lock — a racing caller may have spawned one
    // while we were awaiting the sidecar boot. Drop ours if so.
    let mut client_guard = whisper_state.lock().await;
    if client_guard.is_some() {
        info!("whisper client was spawned concurrently, dropping duplicate");
        drop(client);
        return Ok(());
    }
    *client_guard = Some(client);

    info!("whisper client initialized");
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn shutdown_whisper_client(
    state: tauri::State<'_, WhisperClientState>,
) -> Result<(), CommandError> {
    info!("shutting down whisper client");
    let mut client_guard = state.lock().await;
    if let Some(ref mut client) = *client_guard {
        client.shutdown().await.map_err(CommandError::from)?;
    }
    *client_guard = None;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct WhisperStatusDto {
    pub initialized: bool,
}

/// Reports whether the Whisper sidecar has been initialized.
///
/// The frontend uses this on mount to decide whether to call `init_whisper_client`
/// again. Without it, a Vite HMR remount re-runs autoSetup and respawns the sidecar
/// even when the backend is already warm, leaving `enginePhase: "initializing"`
/// long enough for dictation to fail the readiness check.
#[tauri::command]
#[specta::specta]
pub async fn get_whisper_status(
    state: tauri::State<'_, WhisperClientState>,
) -> Result<WhisperStatusDto, CommandError> {
    let client_guard = state.lock().await;
    Ok(WhisperStatusDto {
        initialized: client_guard.is_some(),
    })
}

/// Locates the sidecar binary relative to the current executable.
fn find_sidecar_path() -> Result<PathBuf, CommandError> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| CommandError::Internal {
            message: format!("failed to get current exe: {e}"),
        })?
        .parent()
        .ok_or(CommandError::Internal {
            message: "failed to get exe directory".into(),
        })?
        .to_path_buf();

    // Tauri bundles sidecar binaries next to the main executable
    let target_triple = current_target_triple();
    let ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    let sidecar_name = format!("yapstack-sidecar-{target_triple}{ext}");

    let path = exe_dir.join(&sidecar_name);
    if path.exists() {
        return Ok(path);
    }

    // Fallback: try without target triple (development mode)
    let fallback_name = format!("yapstack-sidecar{ext}");
    let path = exe_dir.join(&fallback_name);
    if path.exists() {
        return Ok(path);
    }

    Err(CommandError::NotFound {
        message: format!(
            "sidecar binary not found at {} or {}",
            exe_dir.join(&sidecar_name).display(),
            exe_dir.join(&fallback_name).display()
        ),
    })
}

fn current_target_triple() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        }
    } else if cfg!(target_os = "windows") {
        "x86_64-pc-windows-msvc"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_size_dto_roundtrip() {
        let sizes = [
            (ModelSizeDto::Tiny, ModelSize::Tiny),
            (ModelSizeDto::Base, ModelSize::Base),
            (ModelSizeDto::Small, ModelSize::Small),
            (ModelSizeDto::Medium, ModelSize::Medium),
        ];
        for (dto, expected) in sizes {
            let domain: ModelSize = dto.into();
            assert!(
                matches!(domain, ref e if std::mem::discriminant(&domain) == std::mem::discriminant(&expected))
            );
        }
    }

    #[test]
    fn test_model_size_domain_to_dto_roundtrip() {
        let sizes = [
            ModelSize::Tiny,
            ModelSize::Base,
            ModelSize::Small,
            ModelSize::Medium,
        ];
        for size in sizes {
            let dto: ModelSizeDto = size.into();
            let back: ModelSize = dto.into();
            assert_eq!(std::mem::discriminant(&size), std::mem::discriminant(&back));
        }
    }

    #[test]
    fn test_current_target_triple_not_empty() {
        let triple = current_target_triple();
        assert!(!triple.is_empty());
        assert!(triple.contains('-'), "target triple should contain hyphens");
    }
}
