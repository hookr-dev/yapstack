use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::StreamConfig;
use tracing::info;
use yapstack_common::types::PermissionStatus;

use crate::error::AudioError;
use crate::ring_buffer::AudioRingBuffer;
use crate::stream::{build_capture_stream, SendStream};
use crate::DeviceStreamConfig;

type Result<T> = std::result::Result<T, AudioError>;

pub struct SystemAudioCapture {
    stream: Option<SendStream>,
    is_running: bool,
    stream_error: Arc<AtomicBool>,
}

impl SystemAudioCapture {
    pub fn new() -> Self {
        Self {
            stream: None,
            is_running: false,
            stream_error: Arc::new(AtomicBool::new(false)),
        }
    }

    /// System audio capture via cpal loopback is available on macOS 14.2+ and Windows (WASAPI).
    pub fn is_available(&self) -> bool {
        cfg!(any(target_os = "macos", target_os = "windows"))
    }

    /// Queries the default output device's config without starting capture.
    pub fn query_device_config() -> Result<DeviceStreamConfig> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoDevicesAvailable)?;
        let supported = device.default_output_config()?;
        Ok(DeviceStreamConfig {
            sample_rate: supported.sample_rate(),
            channels: supported.channels(),
        })
    }

    pub fn start(&mut self, buffer: Arc<AudioRingBuffer>) -> Result<()> {
        if self.is_running {
            return Err(AudioError::AlreadyRunning);
        }

        if !self.is_available() {
            return Err(AudioError::PlatformNotSupported);
        }

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoDevicesAvailable)?;

        let device_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        // Use the output device's default output config for loopback capture.
        let supported = device.default_output_config()?;

        let stream_config = StreamConfig {
            channels: supported.channels(),
            sample_rate: supported.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        info!(
            "system audio loopback: {}Hz, {}ch (device: {})",
            stream_config.sample_rate, stream_config.channels, device_name
        );

        self.stream_error.store(false, Ordering::Release);
        let stream = build_capture_stream(
            &device,
            &supported,
            &stream_config,
            &buffer,
            "system audio",
            &self.stream_error,
        )?;

        stream.play()?;
        info!(
            "system audio capture started via cpal loopback (device: {})",
            device_name
        );

        self.stream = Some(SendStream::new(stream));
        self.is_running = true;

        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if !self.is_running {
            return Ok(());
        }

        if let Some(stream) = self.stream.take() {
            drop(stream);
        }

        self.is_running = false;
        info!("system audio capture stopped");

        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    /// Returns `true` if the cpal error callback has fired (device disconnect, etc.).
    pub fn has_stream_error(&self) -> bool {
        self.stream_error.load(Ordering::Acquire)
    }

    /// Loopback capture uses standard audio permissions, not screen recording.
    pub fn check_permission(&self) -> PermissionStatus {
        if self.is_available() {
            PermissionStatus::Granted
        } else {
            PermissionStatus::Unavailable
        }
    }
}

impl Default for SystemAudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_system_audio_capture() {
        let capture = SystemAudioCapture::new();
        assert!(!capture.is_running());
    }

    #[test]
    fn test_stop_when_not_running() {
        let mut capture = SystemAudioCapture::new();
        assert!(capture.stop().is_ok());
    }

    #[test]
    fn test_availability() {
        let capture = SystemAudioCapture::new();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert!(capture.is_available());
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert!(!capture.is_available());
    }

    #[test]
    fn test_check_permission() {
        let capture = SystemAudioCapture::new();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert_eq!(capture.check_permission(), PermissionStatus::Granted);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(capture.check_permission(), PermissionStatus::Unavailable);
    }

    #[test]
    #[ignore] // Requires audio hardware
    fn test_start_stop_capture() {
        let mut capture = SystemAudioCapture::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));

        capture.start(buffer).unwrap();
        assert!(capture.is_running());

        capture.stop().unwrap();
        assert!(!capture.is_running());
    }
}
