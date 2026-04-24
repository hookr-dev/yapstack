use serde::Serialize;
use specta::Type;
use std::path::PathBuf;
use tauri::Manager;
use tracing::{error, info};

use super::error::{validate_session_id, CommandError};

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
    let mp3_path = audio_dir.join(format!("{session_id}.mp3"));
    let wav_path = audio_dir.join(format!("{session_id}.wav"));

    if mp3_path.exists() {
        std::fs::remove_file(&mp3_path)?;
    }
    if wav_path.exists() {
        std::fs::remove_file(&wav_path)?;
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
