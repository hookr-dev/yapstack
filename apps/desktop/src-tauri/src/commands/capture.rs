use serde::Serialize;
use specta::Type;
use std::path::PathBuf;
use tauri::Manager;
use tracing::{error, info, warn};

use super::error::{validate_session_id, CommandError};
use crate::{is_allowed_audio_path, AudioBaseOverrideState};

use super::audio::{
    AudioManagerState, CaptureResultDto, CaptureSourceDto, MixConfigDto, SessionStatusDto,
};

#[derive(Debug, Clone, Serialize, Type)]
pub struct SessionWavResultDto {
    pub file_path: String,
    pub duration_seconds: f32,
}

#[tauri::command]
#[specta::specta]
pub async fn trigger_instant_capture(
    state: tauri::State<'_, AudioManagerState>,
    seconds: f32,
    source: CaptureSourceDto,
    mix_config: Option<MixConfigDto>,
) -> Result<CaptureResultDto, CommandError> {
    info!(seconds, source = ?source, "triggering instant capture");
    let manager = state.lock().await;
    let mix = mix_config.map(|c| c.into());
    manager
        .trigger_instant_capture(seconds, source.into(), mix.as_ref())
        .map(|r| {
            info!(
                duration = r.duration_seconds,
                path = %r.file_path.display(),
                "instant capture complete"
            );
            CaptureResultDto::from(r)
        })
        .map_err(|e| {
            error!("instant capture failed: {}", e);
            CommandError::from(e)
        })
}

#[tauri::command]
#[specta::specta]
pub async fn start_session(state: tauri::State<'_, AudioManagerState>) -> Result<(), CommandError> {
    info!("starting session");
    let mut manager = state.lock().await;
    manager.start_session().map_err(|e| {
        error!("start_session failed: {}", e);
        CommandError::from(e)
    })
}

#[tauri::command]
#[specta::specta]
pub async fn end_session(
    state: tauri::State<'_, AudioManagerState>,
    source: CaptureSourceDto,
    mix_config: Option<MixConfigDto>,
) -> Result<CaptureResultDto, CommandError> {
    info!(source = ?source, "ending session");
    let mut manager = state.lock().await;
    let mix = mix_config.map(|c| c.into());
    manager
        .end_session(source.into(), mix.as_ref())
        .map(|r| {
            info!(
                duration = r.duration_seconds,
                path = %r.file_path.display(),
                "session ended"
            );
            CaptureResultDto::from(r)
        })
        .map_err(|e| {
            error!("end_session failed: {}", e);
            CommandError::from(e)
        })
}

#[tauri::command]
#[specta::specta]
pub async fn get_session_status(
    state: tauri::State<'_, AudioManagerState>,
) -> Result<SessionStatusDto, CommandError> {
    let manager = state.lock().await;
    Ok(SessionStatusDto {
        active: manager.is_session_active(),
        elapsed_seconds: manager.session_elapsed_seconds(),
    })
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn export_session_wav(
    state: tauri::State<'_, AudioManagerState>,
    app_handle: tauri::AppHandle,
    session_id: String,
    source: CaptureSourceDto,
    duration_seconds: f32,
    mix_config: Option<MixConfigDto>,
    audio_save_location: Option<String>,
    audio_export_format: Option<String>,
    mp3_bitrate: Option<u16>,
) -> Result<SessionWavResultDto, CommandError> {
    validate_session_id(&session_id)?;
    info!(session_id = %session_id, duration = duration_seconds, "exporting session WAV");

    // Reject invalid MP3 bitrate *before* we read the buffer and write any
    // WAV data; otherwise an invalid kbps would fail inside conversion after
    // the audio is already drained, leaving a stranded WAV on disk.
    let use_mp3 = audio_export_format.as_deref().unwrap_or("mp3") != "wav";
    let effective_bitrate = mp3_bitrate.unwrap_or(64);
    if use_mp3 {
        yapstack_audio::export::validate_mp3_bitrate(effective_bitrate).map_err(|e| {
            CommandError::InvalidInput {
                message: e.to_string(),
            }
        })?;
    }

    let manager = state.lock().await;

    let mix = mix_config.map(|c| c.into());
    let domain_source: yapstack_common::types::CaptureSource = source.into();
    let (samples, sample_rate) = manager
        .extract_source_samples(duration_seconds, domain_source, mix.as_ref())
        .map_err(|_| CommandError::Audio {
            message: match domain_source {
                yapstack_common::types::CaptureSource::MicOnly => "No mic audio available".into(),
                yapstack_common::types::CaptureSource::SystemOnly => {
                    "No system audio available".into()
                }
                yapstack_common::types::CaptureSource::Mixed => "No audio available".into(),
            },
        })?;
    drop(manager);

    let duration = samples.len() as f32 / sample_rate as f32;

    // Write to persistent path
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
    std::fs::create_dir_all(&audio_dir)?;

    let wav_path = audio_dir.join(format!("{session_id}.wav"));
    yapstack_audio::export::write_wav(&samples, sample_rate, 1, &wav_path)
        .map_err(CommandError::from)?;

    let final_path = if use_mp3 {
        yapstack_audio::export::convert_wav_to_mp3(&wav_path, effective_bitrate)
            .map_err(CommandError::from)?
    } else {
        wav_path
    };

    Ok(SessionWavResultDto {
        file_path: final_path.to_string_lossy().into_owned(),
        duration_seconds: duration,
    })
}

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

/// Sets (or clears) a second base directory the `audio-stream://` protocol
/// handler will serve files from, on top of `$APP_DATA_DIR/audio`. Called by
/// the frontend whenever `settings.audioSaveLocation` loads or changes so
/// audio parts written to a custom location remain playable.
#[tauri::command]
#[specta::specta]
pub async fn set_audio_base_override(
    state: tauri::State<'_, AudioBaseOverrideState>,
    path: Option<String>,
) -> Result<(), CommandError> {
    let next = path.map(PathBuf::from);
    let mut guard = state.lock().map_err(|e| CommandError::Internal {
        message: format!("audio base override mutex poisoned: {e}"),
    })?;
    *guard = next;
    Ok(())
}

/// Deletes the listed absolute audio file paths after verifying each lives
/// inside `$APP_DATA_DIR/audio` or under the registered audio-base override.
/// This is what `appStore.deleteSession` uses to clean up a session's parts —
/// parts may live in different directories across the session's lifetime if
/// the user changed `audioSaveLocation` between recording runs, so the FE
/// passes the exact paths from `session_audio_parts`.
#[tauri::command]
#[specta::specta]
pub async fn delete_audio_files(
    app_handle: tauri::AppHandle,
    override_state: tauri::State<'_, AudioBaseOverrideState>,
    paths: Vec<String>,
) -> Result<(), CommandError> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e: tauri::Error| CommandError::Internal {
            message: e.to_string(),
        })?;
    let default_base = app_data_dir.join("audio");
    let override_base = override_state.lock().ok().and_then(|g| g.clone());

    for raw in paths {
        let abs = PathBuf::from(&raw);
        if !abs.is_absolute() {
            warn!(path = %raw, "skipping non-absolute audio file delete");
            continue;
        }
        let allowed_default = is_allowed_audio_path(&default_base, &abs);
        let allowed_override = override_base
            .as_ref()
            .is_some_and(|b| is_allowed_audio_path(b, &abs));
        if !(allowed_default || allowed_override) {
            warn!(path = %raw, "skipping audio file delete outside allowed bases");
            continue;
        }
        match std::fs::remove_file(&abs) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(path = %raw, error = %e, "audio file delete failed"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn test_session_status_dto_inactive() {
        let dto = SessionStatusDto {
            active: false,
            elapsed_seconds: None,
        };
        assert!(!dto.active);
        assert!(dto.elapsed_seconds.is_none());
    }

    #[test]
    fn test_session_status_dto_active() {
        let dto = SessionStatusDto {
            active: true,
            elapsed_seconds: Some(42.5),
        };
        assert!(dto.active);
        assert!((dto.elapsed_seconds.unwrap() - 42.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_session_wav_result_dto_serializes() {
        let dto = SessionWavResultDto {
            file_path: "/tmp/test.wav".to_string(),
            duration_seconds: 10.5,
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("file_path"));
        assert!(json.contains("10.5"));
    }
}
