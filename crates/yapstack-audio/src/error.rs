use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("failed to initialize audio device: {0}")]
    DeviceInit(String),

    #[error("audio capture error: {0}")]
    Capture(String),

    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),

    #[error("no audio devices available")]
    NoDevicesAvailable,

    #[error("device not found: {0}")]
    DeviceNotFound(String),

    #[error("failed to build audio stream: {0}")]
    StreamBuild(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("platform not supported for this operation")]
    PlatformNotSupported,

    #[error("capture is already running")]
    AlreadyRunning,

    #[error("capture is not running")]
    NotRunning,

    #[error("invalid buffer configuration: {0}")]
    InvalidBufferConfig(String),

    #[error("WAV export failed: {0}")]
    WavExport(String),

    #[error("no buffer available")]
    NoBufferAvailable,

    #[error("MP3 encoding failed: {0}")]
    Mp3Encode(String),

    #[error("invalid MP3 bitrate: {0} kbps (allowed: 8, 16, 24, 32, 40, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320)")]
    InvalidBitrate(u16),
}

impl From<cpal::DevicesError> for AudioError {
    fn from(err: cpal::DevicesError) -> Self {
        AudioError::DeviceInit(err.to_string())
    }
}

impl From<cpal::DeviceNameError> for AudioError {
    fn from(err: cpal::DeviceNameError) -> Self {
        AudioError::DeviceInit(err.to_string())
    }
}

impl From<cpal::DeviceIdError> for AudioError {
    fn from(err: cpal::DeviceIdError) -> Self {
        AudioError::DeviceInit(err.to_string())
    }
}

impl From<cpal::DefaultStreamConfigError> for AudioError {
    fn from(err: cpal::DefaultStreamConfigError) -> Self {
        AudioError::DeviceInit(err.to_string())
    }
}

impl From<cpal::BuildStreamError> for AudioError {
    fn from(err: cpal::BuildStreamError) -> Self {
        AudioError::StreamBuild(err.to_string())
    }
}

impl From<cpal::PlayStreamError> for AudioError {
    fn from(err: cpal::PlayStreamError) -> Self {
        AudioError::Capture(err.to_string())
    }
}

impl From<cpal::PauseStreamError> for AudioError {
    fn from(err: cpal::PauseStreamError) -> Self {
        AudioError::Capture(err.to_string())
    }
}

impl From<hound::Error> for AudioError {
    fn from(err: hound::Error) -> Self {
        AudioError::WavExport(err.to_string())
    }
}

impl From<std::io::Error> for AudioError {
    fn from(err: std::io::Error) -> Self {
        AudioError::WavExport(err.to_string())
    }
}
