use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::StreamConfig;
use tracing::info;

use crate::device::resolve_input_device;
use crate::error::AudioError;
use crate::ring_buffer::AudioRingBuffer;
use crate::stream::{build_capture_stream, SendStream};
use crate::DeviceStreamConfig;

type Result<T> = std::result::Result<T, AudioError>;

pub struct MicrophoneCapture {
    stream: Option<SendStream>,
    is_running: bool,
    stream_error: Arc<AtomicBool>,
    /// The device ID used for the current/last capture session.
    last_device_id: Option<String>,
    /// The device name used for the current/last capture session.
    /// Stored on successful start so restarts can target the same device.
    last_device_name: Option<String>,
    /// True when `start()` was called without an explicit `device_id`, so we
    /// intentionally bound whatever the OS default input was at that moment.
    /// Surfaced via `AudioManager::mic_bound_is_default` so the device
    /// broker can distinguish "follow default" from an explicit pick on
    /// `DefaultInputChanged` events: an explicit-but-still-alive pick
    /// keeps its binding, an explicit-but-disappeared pick falls over
    /// to the new default, a follow-default binding always fails over.
    bound_is_default: bool,
}

impl MicrophoneCapture {
    pub fn new() -> Self {
        Self {
            stream: None,
            is_running: false,
            stream_error: Arc::new(AtomicBool::new(false)),
            last_device_id: None,
            last_device_name: None,
            bound_is_default: false,
        }
    }

    /// Queries the device's default input configuration without starting capture.
    pub fn query_device_config(device_id: Option<&str>) -> Result<DeviceStreamConfig> {
        let device = resolve_input_device(device_id, None)?;
        let supported = device.default_input_config()?;
        Ok(DeviceStreamConfig {
            sample_rate: supported.sample_rate(),
            channels: supported.channels(),
        })
    }

    pub fn start(&mut self, device_id: Option<&str>, buffer: Arc<AudioRingBuffer>) -> Result<()> {
        if self.is_running {
            return Err(AudioError::AlreadyRunning);
        }

        let device = resolve_input_device(device_id, None)?;
        let device_actual_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let supported = device.default_input_config()?;

        // Use the device's native config to avoid "unsupported configuration" errors.
        let stream_config = StreamConfig {
            channels: supported.channels(),
            sample_rate: supported.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        info!(
            "using device config: {}Hz, {}ch (device: {})",
            stream_config.sample_rate, stream_config.channels, device_actual_name
        );

        self.stream_error.store(false, Ordering::Release);
        let stream = build_capture_stream(
            &device,
            &supported,
            &stream_config,
            &buffer,
            "microphone",
            &self.stream_error,
        )?;

        stream.play()?;
        info!(
            "microphone capture started on device: {}",
            device_actual_name
        );

        self.stream = Some(SendStream::new(stream));
        self.is_running = true;
        // Persist resolved device info so restarts can target the same device.
        self.last_device_id = device.id().ok().map(|id| id.to_string());
        self.last_device_name = Some(device_actual_name);
        self.bound_is_default = device_id.is_none();

        Ok(())
    }

    /// Returns `true` when the current/last capture bound whatever the OS
    /// default input device was at start-time (i.e. caller passed no device
    /// selection). Callers use this to gate drift-based defenses which are
    /// only meaningful under default-tracking mode.
    pub fn bound_is_default(&self) -> bool {
        self.bound_is_default
    }

    /// Overrides the `bound_is_default` flag. Used by `restart_mic` to
    /// preserve the caller's original tracking intent when a restart
    /// falls through candidates — without this, restarting via the stored
    /// device-id path (even when that device *is* currently the OS default)
    /// would flip the flag off and silently disable the drift check for
    /// the remainder of the session.
    pub fn set_bound_is_default(&mut self, value: bool) {
        self.bound_is_default = value;
    }

    /// Returns the device ID used for the current/last capture session, if any.
    pub fn last_device_id(&self) -> Option<&str> {
        self.last_device_id.as_deref()
    }

    /// Returns the device name used for the current/last capture session, if any.
    pub fn last_device_name(&self) -> Option<&str> {
        self.last_device_name.as_deref()
    }

    pub fn stop(&mut self) -> Result<()> {
        if !self.is_running {
            return Ok(());
        }

        if let Some(stream) = self.stream.take() {
            drop(stream);
        }

        self.is_running = false;
        info!("microphone capture stopped");

        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    /// Returns `true` if the cpal error callback has fired (device disconnect, etc.).
    pub fn has_stream_error(&self) -> bool {
        self.stream_error.load(Ordering::Acquire)
    }
}

impl Default for MicrophoneCapture {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_microphone_capture() {
        let capture = MicrophoneCapture::new();
        assert!(!capture.is_running());
    }

    #[test]
    fn test_stop_when_not_running() {
        let mut capture = MicrophoneCapture::new();
        let result = capture.stop();
        assert!(result.is_ok());
    }

    #[test]
    #[ignore] // Requires audio hardware
    fn test_start_stop_capture() {
        let mut capture = MicrophoneCapture::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));

        capture.start(None, buffer).unwrap();
        assert!(capture.is_running());

        capture.stop().unwrap();
        assert!(!capture.is_running());
    }
}
