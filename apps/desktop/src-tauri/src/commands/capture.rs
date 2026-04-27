use std::path::PathBuf;
use tauri::Manager;

use super::error::{validate_session_id, CommandError};
use crate::audio_dir_trusted;

#[tauri::command]
#[specta::specta]
pub async fn delete_session_wav(
    app_handle: tauri::AppHandle,
    session_id: String,
    audio_save_location: Option<String>,
) -> Result<(), CommandError> {
    validate_session_id(&session_id)?;
    let audio_dir = if let Some(ref custom_dir) = audio_save_location {
        PathBuf::from(custom_dir)
    } else {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e: tauri::Error| CommandError::Internal {
                message: e.to_string(),
            })?;
        app_data_dir.join("audio")
    };

    // Match both the legacy single-file pattern (`{session_id}.wav` —
    // dictations and pre-parts sessions) and the parts pattern
    // (`{session_id}.{part_index}.wav`/`.mp3`).
    let entries = match std::fs::read_dir(&audio_dir) {
        Ok(it) => it,
        Err(_) => return Ok(()), // Audio dir doesn't exist; nothing to delete.
    };
    let prefix = format!("{session_id}.");
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".wav") || lower.ends_with(".mp3")) {
            continue;
        }
        let _ = std::fs::remove_file(entry.path());
    }
    Ok(())
}

/// Deletes the listed absolute audio file paths after verifying each lives
/// in a directory the trusted-audio-dirs set knows about. Used by
/// `appStore.deleteSession` to clean up a session's parts — parts may live
/// in directories across the session's lifetime if the user changed
/// `audioSaveLocation` between recording runs, and every directory we've
/// ever written a part to is in the set.
///
/// Returns `Err(CommandError)` if any path could not be deleted, listing
/// every failed path so the caller can log/toast a useful diagnostic.
#[tauri::command]
#[specta::specta]
pub async fn delete_audio_files(
    app_handle: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<(), CommandError> {
    let mut failures: Vec<String> = Vec::new();
    for raw in paths {
        let abs = PathBuf::from(&raw);
        if !abs.is_absolute() {
            failures.push(format!("{raw} (not absolute)"));
            continue;
        }
        // Authorize by parent directory rather than the file itself so a row
        // that points at an already-deleted file still passes — the trust
        // check shouldn't fail closed just because the target is missing.
        let Some(parent) = abs.parent() else {
            failures.push(format!("{raw} (no parent dir)"));
            continue;
        };
        if !audio_dir_trusted(&app_handle, parent) {
            failures.push(format!("{raw} (outside trusted dirs)"));
            continue;
        }
        match std::fs::remove_file(&abs) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => failures.push(format!("{raw}: {e}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(CommandError::Internal {
            message: format!(
                "failed to delete {} audio file(s): {}",
                failures.len(),
                failures.join("; ")
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::audio::CaptureSourceDto;
    use yapstack_common::types::CaptureSource;

    #[test]
    fn test_capture_source_dto_to_domain_mic_only() {
        let domain: CaptureSource = CaptureSourceDto::MicOnly.into();
        assert!(matches!(domain, CaptureSource::MicOnly));
    }

    #[test]
    fn test_capture_source_dto_to_domain_system_only() {
        let domain: CaptureSource = CaptureSourceDto::SystemOnly.into();
        assert!(matches!(domain, CaptureSource::SystemOnly));
    }

    #[test]
    fn test_capture_source_dto_to_domain_mixed() {
        let domain: CaptureSource = CaptureSourceDto::Mixed.into();
        assert!(matches!(domain, CaptureSource::Mixed));
    }
}
