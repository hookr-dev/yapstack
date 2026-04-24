use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;
use tracing::{error, info};
use yapstack_common::engines::engine_catalogue;
use yapstack_common::types::EngineKind;
use yapstack_transcription::{
    ModelManager, ModelSize, ParakeetVariant, SortformerVariant, TranscriptionClient,
    TranscriptionResult,
};

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
    /// Populated when the active engine is Parakeet *and* diarization
    /// was requested for the originating transcribe call. `None` for Whisper.
    pub speaker_id: Option<u8>,
}

/// Which transcription engine the frontend has selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum EngineKindDto {
    Whisper,
    Parakeet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum ParakeetVariantDto {
    TdtV3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum SortformerVariantDto {
    V2_1,
}

/// Engine capabilities + supported languages for the cascading UI in Settings.
#[derive(Debug, Clone, Serialize, Type)]
pub struct EngineDescriptorDto {
    pub kind: EngineKindDto,
    pub display_name: String,
    pub languages: Vec<String>,
    pub supports_diarization: bool,
    pub supports_initial_prompt: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ParakeetModelInfoDto {
    pub variant: ParakeetVariantDto,
    pub downloaded: bool,
    pub display_name: String,
    pub approximate_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SortformerModelInfoDto {
    pub variant: SortformerVariantDto,
    pub downloaded: bool,
    pub display_name: String,
    pub approximate_size_bytes: u64,
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

impl From<EngineKindDto> for EngineKind {
    fn from(k: EngineKindDto) -> Self {
        match k {
            EngineKindDto::Whisper => EngineKind::Whisper,
            EngineKindDto::Parakeet => EngineKind::Parakeet,
        }
    }
}

impl From<EngineKind> for EngineKindDto {
    fn from(k: EngineKind) -> Self {
        match k {
            EngineKind::Whisper => EngineKindDto::Whisper,
            EngineKind::Parakeet => EngineKindDto::Parakeet,
        }
    }
}

impl From<ParakeetVariantDto> for ParakeetVariant {
    fn from(v: ParakeetVariantDto) -> Self {
        match v {
            ParakeetVariantDto::TdtV3 => ParakeetVariant::TdtV3,
        }
    }
}

impl From<ParakeetVariant> for ParakeetVariantDto {
    fn from(v: ParakeetVariant) -> Self {
        match v {
            ParakeetVariant::TdtV3 => ParakeetVariantDto::TdtV3,
        }
    }
}

impl From<SortformerVariantDto> for SortformerVariant {
    fn from(v: SortformerVariantDto) -> Self {
        match v {
            SortformerVariantDto::V2_1 => SortformerVariant::V2_1,
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
                    speaker_id: s.speaker_id,
                })
                .collect(),
            duration_ms: r.duration_ms,
        }
    }
}

// --- State types ---

pub type ModelManagerState = Arc<Mutex<ModelManager>>;
/// The transcription client is wrapped in `Arc` so `transcribe_audio` can
/// clone a handle out of the state and drop the outer mutex guard before
/// awaiting the sidecar round-trip. `TranscriptionClient` serializes stdin
/// writes internally and demuxes responses via per-request oneshots, so
/// multiple concurrent `&self` calls are safe — the outer mutex only guards
/// initialization and teardown.
pub type TranscriptionClientState = Arc<Mutex<Option<Arc<TranscriptionClient>>>>;

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
    state: tauri::State<'_, TranscriptionClientState>,
    audio_path: String,
    language: Option<String>,
    initial_prompt: Option<String>,
) -> Result<TranscriptionResultDto, CommandError> {
    info!(path = %audio_path, language = ?language, "transcribing audio");
    // Clone the Arc<TranscriptionClient> out of the mutex and release the
    // outer guard immediately. The sidecar round-trip can take seconds;
    // holding the guard across it would serialize every other consumer of
    // the client (shutdown, init, live transcription startup) behind this
    // single transcribe call.
    let client = {
        let client_guard = state.lock().await;
        client_guard.as_ref().cloned()
    }
    .ok_or(CommandError::NotInitialized {
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
pub async fn shutdown_transcription_client(
    state: tauri::State<'_, TranscriptionClientState>,
) -> Result<(), CommandError> {
    info!("shutting down transcription client");
    // Take the Arc out of the state so new `transcribe_audio` calls see None,
    // then request graceful shutdown. `shutdown` takes `&self` (it writes a
    // shutdown request through the internal tokio mutex on stdin), so any
    // concurrent handle held by an in-flight transcribe still sees a clean
    // sidecar wind-down rather than a hard process kill.
    let arc_client = {
        let mut client_guard = state.lock().await;
        client_guard.take()
    };
    if let Some(client) = arc_client {
        client.shutdown().await.map_err(CommandError::from)?;
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct TranscriptionStatusDto {
    pub initialized: bool,
}

/// Reports whether the transcription sidecar has been initialized.
///
/// The frontend uses this on mount to decide whether to call
/// `init_transcription_client` again. Without it, a Vite HMR remount re-runs
/// autoSetup and respawns the sidecar even when the backend is already warm,
/// leaving `enginePhase: "initializing"` long enough for dictation to fail
/// the readiness check.
#[tauri::command]
#[specta::specta]
pub async fn get_transcription_status(
    state: tauri::State<'_, TranscriptionClientState>,
) -> Result<TranscriptionStatusDto, CommandError> {
    let client_guard = state.lock().await;
    Ok(TranscriptionStatusDto {
        initialized: client_guard.is_some(),
    })
}

// ---------- Engine catalogue + Parakeet/Sortformer commands ----------

/// Returns the static engine catalogue (Whisper + Parakeet capability + language lists).
/// The frontend uses this to drive the cascading engine → model → language UI.
#[tauri::command]
#[specta::specta]
pub async fn get_engine_catalogue() -> Result<Vec<EngineDescriptorDto>, CommandError> {
    Ok(engine_catalogue()
        .iter()
        .map(|d| EngineDescriptorDto {
            kind: d.kind.into(),
            display_name: d.display_name.to_string(),
            languages: d.languages.iter().map(|s| (*s).to_string()).collect(),
            supports_diarization: d.supports_diarization,
            supports_initial_prompt: d.supports_initial_prompt,
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn get_parakeet_models(
    state: tauri::State<'_, ModelManagerState>,
) -> Result<Vec<ParakeetModelInfoDto>, CommandError> {
    let manager = state.lock().await;
    Ok(ParakeetVariant::all()
        .iter()
        .copied()
        .map(|v| ParakeetModelInfoDto {
            variant: v.into(),
            downloaded: manager.parakeet_is_available(v),
            display_name: v.display_name().to_string(),
            approximate_size_bytes: v.approximate_size_bytes(),
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn download_parakeet_model(
    state: tauri::State<'_, ModelManagerState>,
    window: tauri::Window,
    variant: ParakeetVariantDto,
) -> Result<String, CommandError> {
    info!(variant = ?variant, "downloading parakeet model");
    let manager = {
        let guard = state.lock().await;
        guard.clone()
    };
    let v: ParakeetVariant = variant.into();
    let path = manager
        .download_parakeet(v, move |progress| {
            let _ = window.emit(
                "model-download-progress",
                serde_json::json!({
                    "percent": progress,
                    "kind": "parakeet",
                    "variant": format!("{:?}", v),
                }),
            );
        })
        .await
        .map_err(|e| {
            error!("parakeet model download failed: {}", e);
            CommandError::from(e)
        })?;
    info!(path = %path.display(), "parakeet model downloaded");
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_parakeet_model(
    state: tauri::State<'_, ModelManagerState>,
    variant: ParakeetVariantDto,
) -> Result<(), CommandError> {
    info!(variant = ?variant, "deleting parakeet model");
    let manager = {
        let guard = state.lock().await;
        guard.clone()
    };
    manager
        .delete_parakeet(variant.into())
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_sortformer_status(
    state: tauri::State<'_, ModelManagerState>,
) -> Result<SortformerModelInfoDto, CommandError> {
    let manager = state.lock().await;
    let v = SortformerVariant::V2_1;
    Ok(SortformerModelInfoDto {
        variant: SortformerVariantDto::V2_1,
        downloaded: manager.sortformer_model_path(v).is_some(),
        display_name: v.display_name().to_string(),
        approximate_size_bytes: v.approximate_size_bytes(),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn download_sortformer_model(
    state: tauri::State<'_, ModelManagerState>,
    window: tauri::Window,
    variant: SortformerVariantDto,
) -> Result<String, CommandError> {
    info!(variant = ?variant, "downloading sortformer model");
    let manager = {
        let guard = state.lock().await;
        guard.clone()
    };
    let v: SortformerVariant = variant.into();
    let path = manager
        .download_sortformer(v, move |progress| {
            let _ = window.emit(
                "model-download-progress",
                serde_json::json!({
                    "percent": progress,
                    "kind": "sortformer",
                    "variant": format!("{:?}", v),
                }),
            );
        })
        .await
        .map_err(|e| {
            error!("sortformer download failed: {}", e);
            CommandError::from(e)
        })?;
    info!(path = %path.display(), "sortformer downloaded");
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_sortformer_model(
    state: tauri::State<'_, ModelManagerState>,
    variant: SortformerVariantDto,
) -> Result<(), CommandError> {
    info!(variant = ?variant, "deleting sortformer model");
    let manager = {
        let guard = state.lock().await;
        guard.clone()
    };
    manager
        .delete_sortformer(variant.into())
        .await
        .map_err(CommandError::from)
}

/// Spawn the sidecar with the requested engine. Whisper requires
/// `whisper_model`; Parakeet requires `parakeet_variant` and may optionally
/// enable Sortformer diarization.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn init_transcription_client(
    app_handle: tauri::AppHandle,
    model_manager_state: tauri::State<'_, ModelManagerState>,
    transcription_state: tauri::State<'_, TranscriptionClientState>,
    engine: EngineKindDto,
    whisper_model: Option<ModelSizeDto>,
    parakeet_variant: Option<ParakeetVariantDto>,
    enable_diarization: bool,
) -> Result<(), CommandError> {
    info!(
        engine = ?engine,
        whisper_model = ?whisper_model,
        parakeet_variant = ?parakeet_variant,
        enable_diarization,
        "initializing transcription client"
    );

    {
        let guard = transcription_state.lock().await;
        if guard.is_some() {
            info!("transcription client already initialized, skipping respawn");
            return Ok(());
        }
    }

    let (model_path, sidecar_path, vad_model_path, sortformer_model_path, coreml_cache_dir) = {
        let manager = model_manager_state.lock().await;
        let sp = find_sidecar_path()?;

        match engine {
            EngineKindDto::Whisper => {
                let size: ModelSize = whisper_model
                    .ok_or(CommandError::NotFound {
                        message: "whisper_model required when engine=Whisper".into(),
                    })?
                    .into();
                let mp = manager.model_path(size).ok_or(CommandError::NotFound {
                    message: format!("whisper model {:?} not downloaded", size),
                })?;
                let vad = match manager.ensure_vad_model().await {
                    Ok(p) => Some(p),
                    Err(e) => {
                        error!("ensure_vad_model failed: {}, proceeding without VAD", e);
                        None
                    }
                };
                (mp, sp, vad, None, None)
            }
            EngineKindDto::Parakeet => {
                let variant: ParakeetVariant = parakeet_variant
                    .ok_or(CommandError::NotFound {
                        message: "parakeet_variant required when engine=Parakeet".into(),
                    })?
                    .into();
                if !manager.parakeet_is_available(variant) {
                    return Err(CommandError::NotFound {
                        message: format!("parakeet variant {:?} not downloaded", variant),
                    });
                }
                let mp = manager.parakeet_model_dir(variant);
                let sortformer = if enable_diarization {
                    Some(manager.ensure_sortformer(SortformerVariant::V2_1).await?)
                } else {
                    None
                };
                let cache_dir = app_handle
                    .path()
                    .app_data_dir()
                    .ok()
                    .map(|d| d.join("cache").join("coreml"));
                (mp, sp, None, sortformer, cache_dir)
            }
        }
    };

    let engine_kind: EngineKind = engine.into();
    let client = TranscriptionClient::spawn(
        &sidecar_path,
        engine_kind,
        &model_path,
        vad_model_path.as_deref(),
        sortformer_model_path.as_deref(),
        coreml_cache_dir.as_deref(),
    )
    .await
    .map_err(|e| {
        error!("failed to spawn transcription client: {}", e);
        CommandError::from(e)
    })?;

    let mut guard = transcription_state.lock().await;
    *guard = Some(Arc::new(client));
    info!(
        "transcription client initialized: engine={}",
        engine_kind.as_str()
    );
    Ok(())
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
