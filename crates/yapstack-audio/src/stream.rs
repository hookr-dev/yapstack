use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cpal::traits::DeviceTrait;
use cpal::{SampleFormat, StreamConfig, SupportedStreamConfig};
use tracing::error;

use crate::error::AudioError;
use crate::ring_buffer::AudioRingBuffer;

/// Wrapper around `cpal::Stream` that implements `Send`.
///
/// `cpal::Stream` is marked `!Send` as a cross-platform safety measure.
/// We gate this wrapper to platforms where the audio backend is known to
/// be safe for single-owner cross-thread moves:
///
/// - **macOS (CoreAudio):** Stream handle is a `CFTypeRef` — thread-safe
///   for ownership transfer. Callbacks run on a dedicated Core Audio thread.
/// - **Windows (WASAPI):** COM-based, initialized with `COINIT_MULTITHREADED`.
///   The WASAPI stream object supports cross-thread ownership.
/// - **Linux (ALSA):** `snd_pcm_t` handle is safe to move between threads
///   when not accessed concurrently (single-owner pattern).
///
/// Invariants:
/// - Single owner: held by `MicrophoneCapture` or `SystemAudioCapture`
/// - Never accessed concurrently — only moved and dropped
/// - Audio callbacks run on OS-managed threads, separate from ownership
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) struct SendStream(#[allow(dead_code)] cpal::Stream);

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!(
    "SendStream requires platform-specific safety audit. \
     See stream.rs for required invariants before adding a new target."
);

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
impl SendStream {
    pub(crate) fn new(stream: cpal::Stream) -> Self {
        Self(stream)
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
// SAFETY: See struct-level documentation for per-platform justification.
// Single-owner, never concurrently accessed. Only moved and dropped.
unsafe impl Send for SendStream {}

/// Builds a cpal input stream for any device + sample format combination,
/// writing audio data into the given ring buffer.
pub(crate) fn build_capture_stream(
    device: &cpal::Device,
    supported: &SupportedStreamConfig,
    stream_config: &StreamConfig,
    buffer: &Arc<AudioRingBuffer>,
    error_label: &'static str,
    stream_error: &Arc<AtomicBool>,
) -> Result<cpal::Stream, AudioError> {
    let stream = match supported.sample_format() {
        SampleFormat::F32 => {
            let buf = Arc::clone(buffer);
            let err_flag = Arc::clone(stream_error);
            device.build_input_stream(
                stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    buf.write(data);
                },
                move |err| {
                    error!("{} stream error: {}", error_label, err);
                    err_flag.store(true, Ordering::Release);
                },
                None,
            )?
        }
        SampleFormat::I16 => {
            let buf = Arc::clone(buffer);
            let err_flag = Arc::clone(stream_error);
            device.build_input_stream(
                stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    buf.write_i16(data);
                },
                move |err| {
                    error!("{} stream error: {}", error_label, err);
                    err_flag.store(true, Ordering::Release);
                },
                None,
            )?
        }
        SampleFormat::U16 => {
            let buf = Arc::clone(buffer);
            let err_flag = Arc::clone(stream_error);
            device.build_input_stream(
                stream_config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    buf.write_u16(data);
                },
                move |err| {
                    error!("{} stream error: {}", error_label, err);
                    err_flag.store(true, Ordering::Release);
                },
                None,
            )?
        }
        format => {
            return Err(AudioError::UnsupportedFormat(format!("{:?}", format)));
        }
    };
    Ok(stream)
}
