use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use yapstack_audio::AudioManager;
use yapstack_audio::BufferPositions;
use yapstack_audio::RingBufferInfo;
use yapstack_common::types;

use super::error::CommandError;

/// Matches the shape of `StreamHealthEvent` in `live_transcription.rs` so the
/// frontend's existing `stream-health` toast handler can surface degradations
/// that happen during initial capture startup (e.g. missing TCC permission for
/// system audio on macOS).
#[derive(Debug, Clone, Serialize)]
struct CaptureStreamHealthEvent {
    source: &'static str,
    status: &'static str,
    message: String,
}

// DTO wrappers that derive specta::Type for IPC boundary
#[derive(Debug, Clone, Serialize, Type)]
pub struct AudioDeviceInfoDto {
    pub id: Option<String>,
    pub name: String,
    pub device_type: DeviceTypeDto,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub enum DeviceTypeDto {
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct CaptureStatusDto {
    pub state: CaptureStateDto,
    pub mic_active: bool,
    pub system_audio_active: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub enum CaptureStateDto {
    Idle,
    Capturing,
    Error,
}

#[derive(Debug, Clone, Serialize, Type)]
pub enum PermissionStatusDto {
    Granted,
    Denied,
    NotDetermined,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct RingBufferInfoDto {
    pub capacity_samples: usize,
    pub samples_written: usize,
    pub available_samples: usize,
    pub capacity_seconds: f32,
    pub available_seconds: f32,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AudioSnapshotDto {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct BufferStatusDto {
    pub mic: Option<RingBufferInfoDto>,
    pub system: Option<RingBufferInfoDto>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CaptureEnergyDto {
    pub mic_rms: Option<f32>,
    pub system_rms: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum CaptureSourceDto {
    MicOnly,
    SystemOnly,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CaptureResultDto {
    pub file_path: String,
    pub duration_seconds: f32,
    pub sample_rate: u32,
    pub source: CaptureSourceDtoSerialize,
}

// Separate serialize-only variant for specta compatibility on the response side
#[derive(Debug, Clone, Serialize, Type)]
pub enum CaptureSourceDtoSerialize {
    MicOnly,
    SystemOnly,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MixConfigDto {
    pub mic_gain: f32,
    pub system_gain: f32,
    pub normalize: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SessionStatusDto {
    pub active: bool,
    pub elapsed_seconds: Option<f32>,
}

impl From<CaptureSourceDto> for types::CaptureSource {
    fn from(s: CaptureSourceDto) -> Self {
        match s {
            CaptureSourceDto::MicOnly => types::CaptureSource::MicOnly,
            CaptureSourceDto::SystemOnly => types::CaptureSource::SystemOnly,
            CaptureSourceDto::Mixed => types::CaptureSource::Mixed,
        }
    }
}

impl From<types::CaptureSource> for CaptureSourceDtoSerialize {
    fn from(s: types::CaptureSource) -> Self {
        match s {
            types::CaptureSource::MicOnly => CaptureSourceDtoSerialize::MicOnly,
            types::CaptureSource::SystemOnly => CaptureSourceDtoSerialize::SystemOnly,
            types::CaptureSource::Mixed => CaptureSourceDtoSerialize::Mixed,
        }
    }
}

impl From<yapstack_audio::CaptureResult> for CaptureResultDto {
    fn from(r: yapstack_audio::CaptureResult) -> Self {
        Self {
            file_path: r.file_path.to_string_lossy().into_owned(),
            duration_seconds: r.duration_seconds,
            sample_rate: r.sample_rate,
            source: r.source.into(),
        }
    }
}

impl MixConfigDto {
    /// Validates gain values, clamping non-finite or negative values to safe defaults.
    fn sanitized(&self) -> (f32, f32) {
        let mic = if self.mic_gain.is_finite() && self.mic_gain >= 0.0 {
            self.mic_gain
        } else {
            1.0
        };
        let sys = if self.system_gain.is_finite() && self.system_gain >= 0.0 {
            self.system_gain
        } else {
            1.0
        };
        (mic, sys)
    }
}

impl From<MixConfigDto> for yapstack_audio::MixConfig {
    fn from(c: MixConfigDto) -> Self {
        let (mic_gain, system_gain) = c.sanitized();
        Self {
            mic_gain,
            system_gain,
            normalize: c.normalize,
        }
    }
}

impl From<types::AudioDeviceInfo> for AudioDeviceInfoDto {
    fn from(d: types::AudioDeviceInfo) -> Self {
        Self {
            id: d.id,
            name: d.name,
            device_type: d.device_type.into(),
            is_default: d.is_default,
        }
    }
}

impl From<types::DeviceType> for DeviceTypeDto {
    fn from(dt: types::DeviceType) -> Self {
        match dt {
            types::DeviceType::Input => DeviceTypeDto::Input,
            types::DeviceType::Output => DeviceTypeDto::Output,
        }
    }
}

impl From<types::CaptureStatus> for CaptureStatusDto {
    fn from(s: types::CaptureStatus) -> Self {
        Self {
            state: s.state.into(),
            mic_active: s.mic_active,
            system_audio_active: s.system_audio_active,
            error_message: s.error_message,
        }
    }
}

impl From<types::CaptureState> for CaptureStateDto {
    fn from(s: types::CaptureState) -> Self {
        match s {
            types::CaptureState::Idle => CaptureStateDto::Idle,
            types::CaptureState::Capturing => CaptureStateDto::Capturing,
            types::CaptureState::Error => CaptureStateDto::Error,
        }
    }
}

impl From<types::PermissionStatus> for PermissionStatusDto {
    fn from(p: types::PermissionStatus) -> Self {
        match p {
            types::PermissionStatus::Granted => PermissionStatusDto::Granted,
            types::PermissionStatus::Denied => PermissionStatusDto::Denied,
            types::PermissionStatus::NotDetermined => PermissionStatusDto::NotDetermined,
            types::PermissionStatus::Unavailable => PermissionStatusDto::Unavailable,
        }
    }
}

impl From<RingBufferInfo> for RingBufferInfoDto {
    fn from(info: RingBufferInfo) -> Self {
        Self {
            capacity_samples: info.capacity_samples,
            samples_written: info.samples_written,
            available_samples: info.available_samples,
            capacity_seconds: info.capacity_seconds,
            available_seconds: info.available_seconds,
            sample_rate: info.sample_rate,
            channels: info.channels,
        }
    }
}

pub type AudioManagerState = Arc<Mutex<AudioManager>>;

#[tauri::command]
#[specta::specta]
pub fn list_audio_devices() -> Result<Vec<AudioDeviceInfoDto>, CommandError> {
    debug!("listing audio devices");
    yapstack_audio::device::list_input_devices()
        .map(|devices| {
            debug!("found {} audio devices", devices.len());
            devices.into_iter().map(AudioDeviceInfoDto::from).collect()
        })
        .map_err(|e| {
            error!("failed to list audio devices: {}", e);
            CommandError::from(e)
        })
}

#[tauri::command]
#[specta::specta]
pub fn get_default_input_device() -> Result<AudioDeviceInfoDto, CommandError> {
    yapstack_audio::device::default_input_device()
        .map(AudioDeviceInfoDto::from)
        .map_err(CommandError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn start_capture(
    app_handle: AppHandle,
    state: tauri::State<'_, AudioManagerState>,
    mic_device_id: Option<String>,
    capture_source: CaptureSourceDto,
    capture_history_seconds: Option<f32>,
) -> Result<(), CommandError> {
    info!(
        source = ?capture_source,
        mic_device_id = ?mic_device_id,
        history_seconds = ?capture_history_seconds,
        "starting capture"
    );
    let mut manager = state.lock().await;

    if let Some(seconds) = capture_history_seconds {
        if seconds <= 0.0 || !seconds.is_finite() {
            return Err(CommandError::InvalidInput {
                message: format!(
                    "capture_history_seconds must be a positive finite number, got {}",
                    seconds
                ),
            });
        }
        let mut config = manager.config().clone();
        config.capture_history_seconds = seconds;
        manager.set_config(config);
    }

    let requested_source: types::CaptureSource = capture_source.into();

    // Idempotent: treat "already running" as success. A Vite HMR remount or
    // StrictMode double-mount will re-fire autoSetup → startCapture, and
    // surfacing AlreadyRunning as an error corrupts the UI's capture status.
    match manager.start_capture(requested_source, mic_device_id.as_deref()) {
        Ok(()) => {
            // In Mixed mode, system-audio failures are swallowed by `start_all`
            // and only surface via `CaptureStatus.error_message` — which the
            // frontend currently toasts only on state transitions to Error.
            // Emit a stream-health event here so the user sees the degradation
            // (e.g. missing screen/system-audio recording permission for the
            // terminal during `npm run dev`, or a Bluetooth output device that
            // cpal loopback can't tap).
            let status = manager.status();
            if matches!(requested_source, types::CaptureSource::Mixed)
                && !status.system_audio_active
            {
                let detail = status
                    .error_message
                    .unwrap_or_else(|| "unknown error".to_string());
                warn!(
                    "start_capture: mixed mode degraded to mic-only — {}",
                    detail
                );
                let _ = app_handle.emit(
                    "stream-health",
                    CaptureStreamHealthEvent {
                        source: "System",
                        status: "restart_failed",
                        message: format!(
                            "System audio unavailable — capturing microphone only. {}",
                            detail
                        ),
                    },
                );
            }
            Ok(())
        }
        Err(yapstack_audio::AudioError::AlreadyRunning) => {
            warn!("start_capture: already running, treating as no-op");
            Ok(())
        }
        Err(e) => {
            error!("start_capture failed: {}", e);
            Err(CommandError::from(e))
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn stop_capture(state: tauri::State<'_, AudioManagerState>) -> Result<(), CommandError> {
    info!("stopping capture");
    let mut manager = state.lock().await;
    manager.stop_all().map_err(|e| {
        error!("stop_capture failed: {}", e);
        CommandError::from(e)
    })
}

#[tauri::command]
#[specta::specta]
pub async fn get_capture_status(
    state: tauri::State<'_, AudioManagerState>,
) -> Result<CaptureStatusDto, CommandError> {
    let manager = state.lock().await;
    Ok(CaptureStatusDto::from(manager.status()))
}

#[tauri::command]
#[specta::specta]
pub async fn check_system_audio_permission(
    state: tauri::State<'_, AudioManagerState>,
) -> Result<PermissionStatusDto, CommandError> {
    let manager = state.lock().await;
    Ok(PermissionStatusDto::from(
        manager.check_system_audio_permission(),
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn snapshot_mic_audio(
    state: tauri::State<'_, AudioManagerState>,
    duration_seconds: Option<f32>,
) -> Result<Option<AudioSnapshotDto>, CommandError> {
    let manager = state.lock().await;

    let samples = match duration_seconds {
        Some(secs) => manager.snapshot_mic(secs),
        None => manager.snapshot_mic_all(),
    };

    Ok(match (samples, manager.mic_buffer_info()) {
        (Some(s), Some(info)) => {
            let duration = s.len() as f32 / (info.sample_rate as f32 * info.channels as f32);
            Some(AudioSnapshotDto {
                samples: s,
                sample_rate: info.sample_rate,
                channels: info.channels,
                duration_seconds: duration,
            })
        }
        _ => None,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn snapshot_system_audio(
    state: tauri::State<'_, AudioManagerState>,
    duration_seconds: Option<f32>,
) -> Result<Option<AudioSnapshotDto>, CommandError> {
    let manager = state.lock().await;

    let samples = match duration_seconds {
        Some(secs) => manager.snapshot_system(secs),
        None => manager.snapshot_system_all(),
    };

    Ok(match (samples, manager.system_buffer_info()) {
        (Some(s), Some(info)) => {
            let duration = s.len() as f32 / (info.sample_rate as f32 * info.channels as f32);
            Some(AudioSnapshotDto {
                samples: s,
                sample_rate: info.sample_rate,
                channels: info.channels,
                duration_seconds: duration,
            })
        }
        _ => None,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn get_buffer_info(
    state: tauri::State<'_, AudioManagerState>,
) -> Result<BufferStatusDto, CommandError> {
    let manager = state.lock().await;
    Ok(BufferStatusDto {
        mic: manager.mic_buffer_info().map(RingBufferInfoDto::from),
        system: manager.system_buffer_info().map(RingBufferInfoDto::from),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn peek_capture_energy(
    state: tauri::State<'_, AudioManagerState>,
    window_secs: f32,
) -> Result<CaptureEnergyDto, CommandError> {
    if window_secs <= 0.0 || !window_secs.is_finite() {
        return Err(CommandError::InvalidInput {
            message: format!(
                "window_secs must be a positive finite number, got {}",
                window_secs
            ),
        });
    }
    let manager = state.lock().await;
    let positions = BufferPositions::default();
    let (mic_rms, system_rms) = manager.peek_energy_rms(&positions, window_secs);
    Ok(CaptureEnergyDto {
        mic_rms,
        system_rms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mix_config_sanitized_valid() {
        let dto = MixConfigDto {
            mic_gain: 0.5,
            system_gain: 0.8,
            normalize: true,
        };
        let (mic, sys) = dto.sanitized();
        assert!((mic - 0.5).abs() < f32::EPSILON);
        assert!((sys - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_mix_config_sanitized_nan() {
        let dto = MixConfigDto {
            mic_gain: f32::NAN,
            system_gain: f32::NAN,
            normalize: false,
        };
        let (mic, sys) = dto.sanitized();
        assert!((mic - 1.0).abs() < f32::EPSILON);
        assert!((sys - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_mix_config_sanitized_infinity() {
        let dto = MixConfigDto {
            mic_gain: f32::INFINITY,
            system_gain: f32::NEG_INFINITY,
            normalize: false,
        };
        let (mic, sys) = dto.sanitized();
        assert!((mic - 1.0).abs() < f32::EPSILON);
        assert!((sys - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_mix_config_sanitized_negative() {
        let dto = MixConfigDto {
            mic_gain: -0.5,
            system_gain: -1.0,
            normalize: true,
        };
        let (mic, sys) = dto.sanitized();
        assert!((mic - 1.0).abs() < f32::EPSILON);
        assert!((sys - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_mix_config_sanitized_zero_valid() {
        let dto = MixConfigDto {
            mic_gain: 0.0,
            system_gain: 0.0,
            normalize: false,
        };
        let (mic, sys) = dto.sanitized();
        assert!((mic - 0.0).abs() < f32::EPSILON);
        assert!((sys - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_mix_config_into_mix_config_sanitizes() {
        let dto = MixConfigDto {
            mic_gain: f32::NAN,
            system_gain: -1.0,
            normalize: true,
        };
        let config: yapstack_audio::MixConfig = dto.into();
        assert!((config.mic_gain - 1.0).abs() < f32::EPSILON);
        assert!((config.system_gain - 1.0).abs() < f32::EPSILON);
        assert!(config.normalize);
    }
}
