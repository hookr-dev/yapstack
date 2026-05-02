//! Tauri commands for the embedding pipeline. Write paths are
//! fire-and-forget from the frontend; the backfill worker catches any
//! that didn't land.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;
use tokio::sync::Mutex;
use tracing::{info, warn};
use yapstack_embedding::{EmbeddingSupervisor, ModelInfo as SupervisorModelInfo};

use super::error::CommandError;
use crate::embedding_db::{EmbeddingStore, SearchHit, SourceKind};

/// Mirrors `commands::transcription::find_sidecar_path`.
pub fn find_embedding_sidecar_path() -> Result<PathBuf, CommandError> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| CommandError::Internal {
            message: format!("failed to get current exe: {e}"),
        })?
        .parent()
        .ok_or(CommandError::Internal {
            message: "failed to get exe directory".into(),
        })?
        .to_path_buf();

    let target_triple = current_target_triple();
    let ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    let suffixed = format!("yapstack-embedding-sidecar-{target_triple}{ext}");
    let path = exe_dir.join(&suffixed);
    if path.exists() {
        return Ok(path);
    }
    let plain = format!("yapstack-embedding-sidecar{ext}");
    let path = exe_dir.join(&plain);
    if path.exists() {
        return Ok(path);
    }
    Err(CommandError::NotFound {
        message: format!(
            "embedding sidecar binary not found at {} or {}",
            exe_dir.join(&suffixed).display(),
            exe_dir.join(&plain).display()
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

/// `supervisor` is `None` until the first `ensure_embedding_ready` (or
/// any other embed_* / search call). Lazy spawn keeps the ~67 MB model
/// off disk for users who have embeddings disabled or non-English set.
pub struct EmbeddingState {
    pub supervisor: Arc<Mutex<Option<EmbeddingSupervisor>>>,
    pub store: Arc<EmbeddingStore>,
    pub cache_dir: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct EmbeddingModelStatusDto {
    /// `true` once the sidecar has finished loading the model and is
    /// ready to embed. Frontend uses this to decide whether to show the
    /// "indexing" indicator.
    pub ready: bool,
    pub model_name: Option<String>,
    pub model_version: Option<String>,
    pub dimensions: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub enum SourceKindDto {
    Segment,
    Dictation,
    Note,
}

impl From<SourceKindDto> for SourceKind {
    fn from(d: SourceKindDto) -> Self {
        match d {
            SourceKindDto::Segment => SourceKind::Segment,
            SourceKindDto::Dictation => SourceKind::Dictation,
            SourceKindDto::Note => SourceKind::Note,
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SemanticHitDto {
    pub source_id: String,
    pub source_kind: SourceKindDto,
    pub distance: f32,
}

/// A dead supervisor is treated as absent and replaced — `EmbeddingSupervisor::respawn`
/// only fires on per-call failures, not on a supervisor that was never
/// alive (e.g. died during model load).
async fn ensure_supervisor_inner(state: &State<'_, EmbeddingState>) -> Result<bool, CommandError> {
    let mut guard = state.supervisor.lock().await;
    if let Some(existing) = guard.as_ref() {
        if existing.is_alive().await {
            return Ok(false);
        }
        warn!("embedding supervisor dead; replacing");
        *guard = None;
    }
    let sidecar_path = find_embedding_sidecar_path()?;
    let supervisor =
        EmbeddingSupervisor::spawn(&sidecar_path, Some(state.cache_dir.as_path())).await?;
    info!("embedding supervisor ready");
    *guard = Some(supervisor);
    Ok(true)
}

async fn supervisor_or_err(
    state: &State<'_, EmbeddingState>,
) -> Result<EmbeddingSupervisor, CommandError> {
    ensure_supervisor_inner(state).await?;
    state
        .supervisor
        .lock()
        .await
        .clone()
        .ok_or_else(|| CommandError::NotInitialized {
            message: "embedding sidecar not spawned".into(),
        })
}

#[tauri::command]
#[specta::specta]
pub async fn ensure_embedding_ready(state: State<'_, EmbeddingState>) -> Result<(), CommandError> {
    ensure_supervisor_inner(&state).await?;
    Ok(())
}

async fn current_model(state: &State<'_, EmbeddingState>) -> Option<SupervisorModelInfo> {
    if let Some(s) = state.supervisor.lock().await.clone() {
        s.model_info().await
    } else {
        None
    }
}

async fn embed_and_store(
    state: State<'_, EmbeddingState>,
    kind: SourceKind,
    source_id: String,
    text: String,
) -> Result<(), CommandError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let supervisor = supervisor_or_err(&state).await?;
    let model = supervisor
        .model_info()
        .await
        .ok_or_else(|| CommandError::NotInitialized {
            message: "embedding model not ready".into(),
        })?;
    // Drop any existing meta/vec for this id before re-embedding. New rows
    // have nothing to delete (no-op). Edited rows leave behind a stale
    // vector if we skipped this and the sidecar call below failed — and
    // backfill's "missing meta" query would never re-attempt them.
    state.store.delete(kind, &source_id)?;
    let vector = supervisor.embed(trimmed.to_string()).await?;
    state.store.upsert(
        kind,
        &source_id,
        trimmed,
        &vector,
        &model.name,
        &model.version,
    )?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn embed_segment(
    state: State<'_, EmbeddingState>,
    segment_id: String,
    text: String,
) -> Result<(), CommandError> {
    embed_and_store(state, SourceKind::Segment, segment_id, text).await
}

#[tauri::command]
#[specta::specta]
pub async fn embed_dictation(
    state: State<'_, EmbeddingState>,
    dictation_id: String,
    text: String,
) -> Result<(), CommandError> {
    embed_and_store(state, SourceKind::Dictation, dictation_id, text).await
}

#[tauri::command]
#[specta::specta]
pub async fn embed_note(
    state: State<'_, EmbeddingState>,
    note_id: String,
    text: String,
) -> Result<(), CommandError> {
    embed_and_store(state, SourceKind::Note, note_id, text).await
}

/// Semantic KNN search across one or more surfaces. `allowed_session_ids`
/// (Some, non-empty) clamps results to those sessions and applies
/// segment lifecycle filters (`hidden = 0 AND deleted_at IS NULL`) at
/// the SQL JOIN level so out-of-scope rows are dropped during the KNN
/// join, not after — otherwise tightly-scoped chats can return empty
/// when most top-k hits are out of scope.
#[tauri::command]
#[specta::specta]
pub async fn search_semantic(
    state: State<'_, EmbeddingState>,
    query: String,
    k: u32,
    kinds: Vec<SourceKindDto>,
    allowed_session_ids: Option<Vec<String>>,
) -> Result<Vec<SemanticHitDto>, CommandError> {
    if query.trim().is_empty() || k == 0 || kinds.is_empty() {
        return Ok(vec![]);
    }
    let q_preview: String = query.chars().take(80).collect();
    let scope_n = allowed_session_ids.as_ref().map(|v| v.len());
    let supervisor = supervisor_or_err(&state).await?;
    let qvec = supervisor.embed_query(query).await?;
    let store = Arc::clone(&state.store);
    let allow_slice = allowed_session_ids.as_deref();
    let mut hits: Vec<SemanticHitDto> = Vec::new();
    for k_dto in kinds.iter().copied() {
        let kind: SourceKind = k_dto.into();
        let res = store.search(kind, &qvec, k as usize, allow_slice)?;
        hits.extend(res.into_iter().map(
            |SearchHit {
                 source_id,
                 distance,
             }| {
                SemanticHitDto {
                    source_id,
                    source_kind: k_dto,
                    distance,
                }
            },
        ));
    }
    hits.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(k as usize);
    let top: Vec<String> = hits
        .iter()
        .take(5)
        .map(|h| format!("{:?}:{}={:.3}", h.source_kind, h.source_id, h.distance))
        .collect();
    info!(
        "search_semantic q=\"{}\" surfaces={:?} scope={:?} k={} hits={} top=[{}]",
        q_preview,
        kinds,
        scope_n,
        k,
        hits.len(),
        top.join(", ")
    );
    Ok(hits)
}

#[tauri::command]
#[specta::specta]
pub async fn embedding_model_status(
    state: State<'_, EmbeddingState>,
) -> Result<EmbeddingModelStatusDto, CommandError> {
    match current_model(&state).await {
        Some(info) => Ok(EmbeddingModelStatusDto {
            ready: true,
            model_name: Some(info.name),
            model_version: Some(info.version),
            dimensions: Some(info.dimensions),
        }),
        None => Ok(EmbeddingModelStatusDto {
            ready: false,
            model_name: None,
            model_version: None,
            dimensions: None,
        }),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn delete_segment_embedding(
    state: State<'_, EmbeddingState>,
    segment_id: String,
) -> Result<(), CommandError> {
    state.store.delete(SourceKind::Segment, &segment_id)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_dictation_embedding(
    state: State<'_, EmbeddingState>,
    dictation_id: String,
) -> Result<(), CommandError> {
    state.store.delete(SourceKind::Dictation, &dictation_id)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_note_embedding(
    state: State<'_, EmbeddingState>,
    note_id: String,
) -> Result<(), CommandError> {
    state.store.delete(SourceKind::Note, &note_id)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_session_embeddings(
    state: State<'_, EmbeddingState>,
    session_id: String,
) -> Result<u32, CommandError> {
    let removed = state.store.delete_by_session(&session_id)?;
    Ok(removed as u32)
}

/// Returns up to `batch_size` source rows (id + text) that lack an
/// embedding for the given surface. Used by the frontend backfill worker
/// to pull work batches.
#[tauri::command]
#[specta::specta]
pub async fn list_missing_embeddings(
    state: State<'_, EmbeddingState>,
    kind: SourceKindDto,
    batch_size: u32,
) -> Result<Vec<MissingRowDto>, CommandError> {
    let kind: SourceKind = kind.into();
    let rows = state.store.missing(kind, batch_size as usize)?;
    Ok(rows
        .into_iter()
        .map(|(id, text)| MissingRowDto { id, text })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MissingRowDto {
    pub id: String,
    pub text: String,
}

/// Batch embed + upsert. Used by backfill — frontend collects a batch via
/// `list_missing_embeddings`, then hands it back here. We embed in one
/// `embed_batch` round-trip, then upsert each result.
#[tauri::command]
#[specta::specta]
pub async fn embed_and_store_batch(
    state: State<'_, EmbeddingState>,
    kind: SourceKindDto,
    rows: Vec<MissingRowDto>,
) -> Result<u32, CommandError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let kind: SourceKind = kind.into();
    let supervisor = supervisor_or_err(&state).await?;
    let model = supervisor
        .model_info()
        .await
        .ok_or_else(|| CommandError::NotInitialized {
            message: "embedding model not ready".into(),
        })?;
    let texts: Vec<String> = rows.iter().map(|r| r.text.clone()).collect();
    let vectors = supervisor.embed_batch(texts).await?;
    let mut written = 0u32;
    for (row, vector) in rows.into_iter().zip(vectors) {
        match state.store.upsert(
            kind,
            &row.id,
            &row.text,
            &vector,
            &model.name,
            &model.version,
        ) {
            Ok(true) => written += 1,
            Ok(false) => {}
            Err(e) => warn!("embed_and_store_batch upsert failed for {}: {}", row.id, e),
        }
    }
    Ok(written)
}
