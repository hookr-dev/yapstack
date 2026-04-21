//! Tauri commands for the in-app log viewer.
//!
//! See `crate::logging` for the subscriber that populates the buffer and
//! writes the rotating file on disk.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use super::error::CommandError;
use crate::logging::{LogBuffer, LogEntry};

/// Snapshot of recent entries from the in-memory ring buffer.
/// `limit` caps the number returned (default 500 = the full buffer).
#[tauri::command]
#[specta::specta]
pub async fn get_recent_logs(
    limit: Option<usize>,
    buffer: State<'_, Arc<LogBuffer>>,
) -> Result<Vec<LogEntry>, CommandError> {
    Ok(buffer.snapshot(limit.unwrap_or(500)))
}

/// Clear the in-memory ring buffer. Does not touch on-disk log files.
#[tauri::command]
#[specta::specta]
pub async fn clear_logs(buffer: State<'_, Arc<LogBuffer>>) -> Result<(), CommandError> {
    buffer.clear();
    Ok(())
}

/// Return the resolved log directory path as a string, for display in the UI.
#[tauri::command]
#[specta::specta]
pub async fn get_log_dir(app: AppHandle) -> Result<String, CommandError> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|e| CommandError::Internal {
            message: format!("failed to resolve log dir: {e}"),
        })?;
    Ok(dir.to_string_lossy().into_owned())
}

/// Open Finder / Explorer to the log directory so the user can grab the files
/// to send to support.
#[tauri::command]
#[specta::specta]
pub async fn reveal_log_dir(app: AppHandle) -> Result<(), CommandError> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|e| CommandError::Internal {
            message: format!("failed to resolve log dir: {e}"),
        })?;
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| CommandError::Internal {
            message: format!("failed to open log dir: {e}"),
        })
}
