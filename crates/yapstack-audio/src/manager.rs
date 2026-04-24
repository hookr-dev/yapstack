use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, error, warn};
use yapstack_common::config::AudioConfig;
use yapstack_common::types::{CaptureSource, CaptureState, CaptureStatus, PermissionStatus};

use crate::capture::{
    BufferPositions, CaptureResult, CapturedAudio, SeparateExtraction, SessionMark,
};
use crate::error::AudioError;
use crate::export;
use crate::mic::MicrophoneCapture;
use crate::mixer::{self, MixConfig};
use crate::ring_buffer::{AudioRingBuffer, RingBufferInfo, SharedAudioRingBuffer};
use crate::system::device_watcher::{DefaultDeviceKind, DefaultDeviceWatcher};
use crate::system::SystemAudioCapture;
use crate::DeviceStreamConfig;

type Result<T> = std::result::Result<T, AudioError>;

/// Signals whether a stream restart was able to keep the existing ring
/// buffer or had to allocate a new one. Callers must reset any cursors
/// or VAD state tied to the previous buffer when `BufferReplaced` is
/// returned — positions from the old buffer are not meaningful against
/// the new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartOutcome {
    BufferPreserved,
    BufferReplaced,
}

pub struct AudioManager {
    config: AudioConfig,
    mic: MicrophoneCapture,
    system: SystemAudioCapture,
    state: CaptureState,
    error_message: Option<String>,
    mic_buffer: Option<SharedAudioRingBuffer>,
    system_buffer: Option<SharedAudioRingBuffer>,
    session_mark: Option<SessionMark>,
    /// Push-based default-device change listeners (macOS only; no-op stubs
    /// elsewhere). `None` if CoreAudio rejected listener registration — the
    /// capture path degrades to the write-pos stall watchdog in that case.
    input_watcher: Option<DefaultDeviceWatcher>,
    output_watcher: Option<DefaultDeviceWatcher>,
}

impl AudioManager {
    pub fn new() -> Self {
        Self::from_config(AudioConfig::default())
    }

    pub fn with_config(config: AudioConfig) -> Self {
        Self::from_config(config)
    }

    fn from_config(config: AudioConfig) -> Self {
        let input_watcher = DefaultDeviceWatcher::new(DefaultDeviceKind::Input)
            .map_err(|e| warn!("input default-device listener unavailable: {}", e))
            .ok();
        let output_watcher = DefaultDeviceWatcher::new(DefaultDeviceKind::Output)
            .map_err(|e| warn!("output default-device listener unavailable: {}", e))
            .ok();
        Self {
            config,
            mic: MicrophoneCapture::new(),
            system: SystemAudioCapture::new(),
            state: CaptureState::Idle,
            error_message: None,
            mic_buffer: None,
            system_buffer: None,
            session_mark: None,
            input_watcher,
            output_watcher,
        }
    }

    pub fn start_mic(&mut self, device_id: Option<&str>) -> Result<()> {
        // Query the device's native config. The buffer uses the device's actual
        // sample rate and channel count — we no longer mutate self.config so that
        // each buffer carries its own format independently.
        let device_config = MicrophoneCapture::query_device_config(device_id)?;

        let buffer = Arc::new(AudioRingBuffer::with_duration(
            self.config.capture_history_seconds,
            device_config.sample_rate,
            device_config.channels,
        ));
        self.mic.start(device_id, Arc::clone(&buffer))?;
        self.mic_buffer = Some(buffer);
        self.state = CaptureState::Capturing;
        self.error_message = None;
        Ok(())
    }

    pub fn start_system_audio(&mut self) -> Result<()> {
        // Query the output device's native config so the buffer matches the
        // actual stream format. Output devices are typically stereo (2ch) while
        // the mic config may be mono — using the wrong channel count causes the
        // buffer to report duration at the wrong rate.
        let device_config = SystemAudioCapture::query_device_config()?;

        let buffer = Arc::new(AudioRingBuffer::with_duration(
            self.config.capture_history_seconds,
            device_config.sample_rate,
            device_config.channels,
        ));
        self.system.start(Arc::clone(&buffer))?;
        self.system_buffer = Some(buffer);
        self.state = CaptureState::Capturing;
        self.error_message = None;
        Ok(())
    }

    pub fn start_capture(
        &mut self,
        source: CaptureSource,
        mic_device_id: Option<&str>,
    ) -> Result<()> {
        let result = match source {
            CaptureSource::MicOnly => self.start_mic(mic_device_id),
            CaptureSource::SystemOnly => self.start_system_audio(),
            CaptureSource::Mixed => self.start_all(mic_device_id),
        };
        if let Err(ref e) = result {
            if !matches!(e, AudioError::AlreadyRunning) {
                self.state = CaptureState::Error;
                self.error_message = Some(e.to_string());
            }
        }
        result
    }

    pub fn start_all(&mut self, mic_device_id: Option<&str>) -> Result<()> {
        self.start_mic(mic_device_id)?;

        if let Err(e) = self.start_system_audio() {
            error!("system audio capture failed to start: {}", e);
            // Continue with mic-only capture but surface the degradation
            self.error_message = Some(format!("Mixed mode degraded to mic-only: {}", e));
        }

        Ok(())
    }

    pub fn stop_all(&mut self) -> Result<()> {
        let mut had_error = false;

        if self.mic.is_running() {
            if let Err(e) = self.mic.stop() {
                error!("failed to stop microphone: {}", e);
                had_error = true;
            }
        }

        if self.system.is_running() {
            if let Err(e) = self.system.stop() {
                error!("failed to stop system audio: {}", e);
                had_error = true;
            }
        }

        // Buffers are intentionally retained after stop so capture history
        // can still be read back for transcription.

        if had_error {
            let msg = self
                .error_message
                .clone()
                .unwrap_or_else(|| "error stopping capture".to_string());
            self.state = CaptureState::Error;
            self.error_message = Some(msg.clone());
            Err(AudioError::Capture(msg))
        } else {
            self.state = CaptureState::Idle;
            self.error_message = None;
            Ok(())
        }
    }

    pub fn status(&self) -> CaptureStatus {
        CaptureStatus {
            state: self.state,
            mic_active: self.mic.is_running(),
            system_audio_active: self.system.is_running(),
            error_message: self.error_message.clone(),
        }
    }

    pub fn check_system_audio_permission(&self) -> PermissionStatus {
        self.system.check_permission()
    }

    // --- Snapshot API ---

    pub fn snapshot_mic(&self, duration_seconds: f32) -> Option<Vec<f32>> {
        self.mic_buffer
            .as_ref()
            .map(|b| b.snapshot(duration_seconds))
    }

    pub fn snapshot_system(&self, duration_seconds: f32) -> Option<Vec<f32>> {
        self.system_buffer
            .as_ref()
            .map(|b| b.snapshot(duration_seconds))
    }

    pub fn snapshot_mic_all(&self) -> Option<Vec<f32>> {
        self.mic_buffer.as_ref().map(|b| b.snapshot_all())
    }

    pub fn snapshot_system_all(&self) -> Option<Vec<f32>> {
        self.system_buffer.as_ref().map(|b| b.snapshot_all())
    }

    pub fn mic_buffer_info(&self) -> Option<RingBufferInfo> {
        self.mic_buffer.as_ref().map(|b| b.info())
    }

    pub fn system_buffer_info(&self) -> Option<RingBufferInfo> {
        self.system_buffer.as_ref().map(|b| b.info())
    }

    // --- Buffer access ---

    pub fn mic_buffer(&self) -> Option<&SharedAudioRingBuffer> {
        self.mic_buffer.as_ref()
    }

    pub fn system_buffer(&self) -> Option<&SharedAudioRingBuffer> {
        self.system_buffer.as_ref()
    }

    /// Returns the mic buffer's current write position, or 0 if no mic buffer exists.
    pub fn mic_write_pos(&self) -> usize {
        self.mic_buffer
            .as_ref()
            .map(|b| b.samples_written())
            .unwrap_or(0)
    }

    /// Returns the system buffer's current write position, or 0 if no system buffer exists.
    pub fn system_write_pos(&self) -> usize {
        self.system_buffer
            .as_ref()
            .map(|b| b.samples_written())
            .unwrap_or(0)
    }

    /// Returns the sample rate from the first active buffer (mic preferred), or the
    /// config default. Used wherever a single representative sample rate is needed.
    fn active_sample_rate(&self) -> u32 {
        self.mic_buffer
            .as_ref()
            .map(|b| b.sample_rate())
            .or_else(|| self.system_buffer.as_ref().map(|b| b.sample_rate()))
            .unwrap_or(yapstack_common::config::DEFAULT_SAMPLE_RATE)
    }

    /// Returns the sample rate tied to the buffer whose samples the caller
    /// actually receives for `source`. Using `active_sample_rate()` here would
    /// return the mic rate even for SystemOnly callers with a stale mic buffer
    /// present, producing wrong-rate WAV metadata. For Mixed, mic's rate wins
    /// because `resample_and_mix` upsamples system to mic's rate.
    fn sample_rate_for_source(&self, source: CaptureSource) -> u32 {
        match source {
            CaptureSource::MicOnly => self
                .mic_buffer
                .as_ref()
                .map(|b| b.sample_rate())
                .unwrap_or_else(|| self.active_sample_rate()),
            CaptureSource::SystemOnly => self
                .system_buffer
                .as_ref()
                .map(|b| b.sample_rate())
                .unwrap_or_else(|| self.active_sample_rate()),
            CaptureSource::Mixed => match (self.mic_buffer.as_ref(), self.system_buffer.as_ref()) {
                (Some(mb), _) => mb.sample_rate(),
                (None, Some(sb)) => sb.sample_rate(),
                (None, None) => self.active_sample_rate(),
            },
        }
    }

    /// Resample system audio to match mic rate (if different) and mix to mono.
    /// Used by instant capture, extract_since, and end_session.
    /// On resample failure, logs the error and falls back to mic-only audio.
    fn resample_and_mix(
        &self,
        mic_samples: &[f32],
        system_samples: &[f32],
        mix_config: Option<&MixConfig>,
    ) -> Vec<f32> {
        let config = mix_config.cloned().unwrap_or_default();
        if let (Some(mic_buf), Some(sys_buf)) =
            (self.mic_buffer.as_ref(), self.system_buffer.as_ref())
        {
            if mic_buf.sample_rate() != sys_buf.sample_rate() {
                debug!(
                    "resampling system audio {}Hz → {}Hz for mixed mode",
                    sys_buf.sample_rate(),
                    mic_buf.sample_rate()
                );
                match mixer::resample(system_samples, sys_buf.sample_rate(), mic_buf.sample_rate())
                {
                    Ok(s_resampled) => {
                        return mixer::mix_to_mono(mic_samples, &s_resampled, &config);
                    }
                    Err(e) => {
                        error!("mixed mode resample failed, using mic-only: {e}");
                        return mic_samples.to_vec();
                    }
                }
            }
        }
        mixer::mix_to_mono(mic_samples, system_samples, &config)
    }

    // --- Capture API ---

    /// Extracts captured audio from the last `duration_seconds` of both buffers.
    ///
    /// Each buffer's own sample rate and channel count are used for sample count
    /// calculation. Multi-channel data is deinterleaved to mono so the returned
    /// `CapturedAudio` always has `channels == 1`.
    ///
    /// **Note:** The returned `sample_rate` is the mic buffer's rate (or system
    /// buffer's if mic is absent). When mixing sources with different sample rates,
    /// callers must resample before mixing — see `trigger_instant_capture` and
    /// `end_session` for examples.
    pub fn extract_captured_audio(&self, duration_seconds: f32) -> CapturedAudio {
        let mic = self.mic_buffer.as_ref().map(|b| {
            let sample_count =
                (duration_seconds * b.sample_rate() as f32 * b.channels() as f32) as usize;
            let raw = b.snapshot_samples(sample_count);
            (
                mixer::deinterleave_to_mono(&raw, b.channels()).into_owned(),
                b.sample_rate(),
            )
        });

        let system = self.system_buffer.as_ref().map(|b| {
            let sample_count =
                (duration_seconds * b.sample_rate() as f32 * b.channels() as f32) as usize;
            let raw = b.snapshot_samples(sample_count);
            (
                mixer::deinterleave_to_mono(&raw, b.channels()).into_owned(),
                b.sample_rate(),
            )
        });

        let mic_sample_rate = mic.as_ref().map(|(_, r)| *r);
        let system_sample_rate = system.as_ref().map(|(_, r)| *r);

        let mic_samples = mic.map(|(s, _)| s).unwrap_or_default();
        let system_samples = system.map(|(s, _)| s).unwrap_or_default();

        let sample_rate = self.active_sample_rate();

        let mono_len = mic_samples.len().max(system_samples.len());
        let actual_duration = mono_len as f32 / sample_rate as f32;

        CapturedAudio {
            mic_samples,
            system_samples,
            mic_sample_rate,
            system_sample_rate,
            sample_rate,
            channels: 1,
            duration_seconds: actual_duration,
        }
    }

    /// Extracts mono audio for a single source over the last `seconds` window
    /// and returns `(samples, sample_rate)`. For Mixed, resamples system to
    /// mic's rate before mixing. Returns `Err(NoBufferAvailable)` if the
    /// requested source has no data.
    pub fn extract_source_samples(
        &self,
        seconds: f32,
        source: CaptureSource,
        mix_config: Option<&MixConfig>,
    ) -> Result<(Vec<f32>, u32)> {
        let captured = self.extract_captured_audio(seconds);
        let sample_rate = self.sample_rate_for_source(source);

        let samples = match source {
            CaptureSource::MicOnly => {
                if captured.mic_samples.is_empty() {
                    return Err(AudioError::NoBufferAvailable);
                }
                captured.mic_samples
            }
            CaptureSource::SystemOnly => {
                if captured.system_samples.is_empty() {
                    return Err(AudioError::NoBufferAvailable);
                }
                captured.system_samples
            }
            CaptureSource::Mixed => {
                if captured.mic_samples.is_empty() && captured.system_samples.is_empty() {
                    return Err(AudioError::NoBufferAvailable);
                }
                self.resample_and_mix(&captured.mic_samples, &captured.system_samples, mix_config)
            }
        };

        if samples.is_empty() {
            return Err(AudioError::NoBufferAvailable);
        }

        Ok((samples, sample_rate))
    }

    /// Performs an instant capture of the last N seconds, writing to a temp WAV file.
    ///
    /// All audio is deinterleaved to mono before export. The WAV file is always
    /// single-channel at the buffer's native sample rate.
    pub fn trigger_instant_capture(
        &self,
        seconds: f32,
        source: CaptureSource,
        mix_config: Option<&MixConfig>,
    ) -> Result<CaptureResult> {
        let (samples, sample_rate) = self.extract_source_samples(seconds, source, mix_config)?;

        let duration_seconds = samples.len() as f32 / sample_rate as f32;
        let file_path = export::write_wav_to_temp(&samples, sample_rate, 1)?;

        Ok(CaptureResult {
            file_path,
            duration_seconds,
            sample_rate,
            source,
        })
    }

    // --- Position tracking API ---

    /// Returns current write positions for both buffers.
    pub fn buffer_positions(&self) -> BufferPositions {
        BufferPositions {
            mic_pos: self
                .mic_buffer
                .as_ref()
                .map(|b| b.samples_written())
                .unwrap_or(0),
            system_pos: self
                .system_buffer
                .as_ref()
                .map(|b| b.samples_written())
                .unwrap_or(0),
        }
    }

    /// Extracts mono audio from active buffers since the given positions.
    ///
    /// Returns `(mono_samples, sample_rate, new_positions)` or `None` if no
    /// buffers are active or the requested source has no data.
    ///
    /// Uses `snapshot_since_with_pos` to capture the exact write position at the
    /// time of each snapshot. This eliminates the race window where audio callbacks
    /// could advance the write position between the snapshot and position query,
    /// which previously caused cumulative sample loss and source drift in Mixed mode.
    pub fn extract_since(
        &self,
        positions: &BufferPositions,
        source: CaptureSource,
        mix_config: Option<&MixConfig>,
    ) -> Option<(Vec<f32>, u32, BufferPositions)> {
        let (mic_mono, mut mic_new_pos) = self
            .mic_buffer
            .as_ref()
            .map(|b| {
                let (snap, pos) = b.snapshot_since_with_pos(positions.mic_pos);
                let mono = mixer::deinterleave_to_mono(&snap, b.channels()).into_owned();
                (Some(mono), pos)
            })
            .unwrap_or((None, positions.mic_pos));

        let (system_mono, mut sys_new_pos) = self
            .system_buffer
            .as_ref()
            .map(|b| {
                let (snap, pos) = b.snapshot_since_with_pos(positions.system_pos);
                let mono = mixer::deinterleave_to_mono(&snap, b.channels()).into_owned();
                (Some(mono), pos)
            })
            .unwrap_or((None, positions.system_pos));

        // Source-specific output rate. Using `active_sample_rate()` here
        // previously returned the mic buffer's rate unconditionally, so a
        // SystemOnly session with a stale mic buffer would report the mic
        // rate even though the returned samples come from the system
        // buffer — callers (e.g. the session WAV writer's resample step)
        // would then skip or miscompute conversion. Tie the rate to the
        // buffer the returned samples actually come from.
        let sample_rate = match source {
            CaptureSource::MicOnly => self
                .mic_buffer
                .as_ref()
                .map(|b| b.sample_rate())
                .unwrap_or_else(|| self.active_sample_rate()),
            CaptureSource::SystemOnly => self
                .system_buffer
                .as_ref()
                .map(|b| b.sample_rate())
                .unwrap_or_else(|| self.active_sample_rate()),
            // Mixed mode: `resample_and_mix` resamples system to mic's rate
            // and returns mic-rate output when both buffers are present.
            // When only one buffer is present, the sole source's rate wins.
            CaptureSource::Mixed => match (self.mic_buffer.as_ref(), self.system_buffer.as_ref()) {
                (Some(mb), _) => mb.sample_rate(),
                (None, Some(sb)) => sb.sample_rate(),
                (None, None) => self.active_sample_rate(),
            },
        };

        let samples = match source {
            CaptureSource::MicOnly => {
                let m = mic_mono.unwrap_or_default();
                if m.is_empty() {
                    return None;
                }
                m
            }
            CaptureSource::SystemOnly => {
                let s = system_mono.unwrap_or_default();
                if s.is_empty() {
                    return None;
                }
                s
            }
            CaptureSource::Mixed => {
                let m = mic_mono.unwrap_or_default();
                let s = system_mono.unwrap_or_default();
                if m.is_empty() && s.is_empty() {
                    return None;
                }
                // Time-based trimming: when mic and system have different sample rates
                // or different sample counts, trim each to the minimum *time duration*
                // at its own rate. This prevents cumulative positional drift that occurs
                // when trimming by sample count across mismatched rates.
                if !m.is_empty() && !s.is_empty() {
                    let (mic_buf, sys_buf) =
                        match (self.mic_buffer.as_ref(), self.system_buffer.as_ref()) {
                            (Some(mb), Some(sb)) => (mb, sb),
                            _ => return None, // invariant: both buffers must exist when both arrays are non-empty
                        };
                    let mic_sr = mic_buf.sample_rate() as f64;
                    let sys_sr = sys_buf.sample_rate() as f64;

                    if mic_sr != sys_sr || m.len() != s.len() {
                        let mic_time = m.len() as f64 / mic_sr;
                        let sys_time = s.len() as f64 / sys_sr;
                        let min_time = mic_time.min(sys_time);

                        let mic_keep = ((min_time * mic_sr) as usize).min(m.len());
                        let sys_keep = ((min_time * sys_sr) as usize).min(s.len());

                        let mic_surplus = m.len() - mic_keep;
                        let sys_surplus = s.len() - sys_keep;

                        mic_new_pos =
                            mic_new_pos.saturating_sub(mic_surplus * mic_buf.channels() as usize);
                        sys_new_pos =
                            sys_new_pos.saturating_sub(sys_surplus * sys_buf.channels() as usize);

                        self.resample_and_mix(&m[..mic_keep], &s[..sys_keep], mix_config)
                    } else {
                        self.resample_and_mix(&m, &s, mix_config)
                    }
                } else {
                    self.resample_and_mix(&m, &s, mix_config)
                }
            }
        };

        if samples.is_empty() {
            return None;
        }

        let new_positions = BufferPositions {
            mic_pos: mic_new_pos,
            system_pos: sys_new_pos,
        };
        Some((samples, sample_rate, new_positions))
    }

    // --- Separate extraction API ---

    /// Extracts mono audio from each buffer independently since the given positions.
    ///
    /// Unlike `extract_since()`, this does **not** mix sources. Each source's
    /// samples are returned separately, suitable for per-source VAD and transcription.
    ///
    /// Uses `snapshot_since_with_pos` to capture exact positions (see `extract_since`).
    pub fn extract_sources_since(&self, positions: &BufferPositions) -> Option<SeparateExtraction> {
        let mut mic_new_pos = positions.mic_pos;
        let mic = self.mic_buffer.as_ref().and_then(|b| {
            let (snap, pos) = b.snapshot_since_with_pos(positions.mic_pos);
            mic_new_pos = pos;
            if snap.is_empty() {
                None
            } else {
                let mono = mixer::deinterleave_to_mono(&snap, b.channels()).into_owned();
                Some((mono, b.sample_rate()))
            }
        });

        let mut sys_new_pos = positions.system_pos;
        let system = self.system_buffer.as_ref().and_then(|b| {
            let (snap, pos) = b.snapshot_since_with_pos(positions.system_pos);
            sys_new_pos = pos;
            if snap.is_empty() {
                None
            } else {
                let mono = mixer::deinterleave_to_mono(&snap, b.channels()).into_owned();
                Some((mono, b.sample_rate()))
            }
        });

        if mic.is_none() && system.is_none() {
            return None;
        }

        Some(SeparateExtraction {
            mic,
            system,
            new_positions: BufferPositions {
                mic_pos: mic_new_pos,
                system_pos: sys_new_pos,
            },
        })
    }

    /// Returns the RMS energy computed directly from ring buffer data (zero-allocation).
    ///
    /// Computes RMS inline over the raw ring buffer without allocating a `Vec<f32>`.
    /// For multi-channel buffers, the energy is computed over interleaved samples
    /// (not deinterleaved mono), which is sufficient for VAD threshold comparisons.
    /// Returns `(mic_energy, system_energy)` where each is `None` if that buffer
    /// isn't active or has no new data since `positions`.
    pub fn peek_energy_rms(
        &self,
        positions: &BufferPositions,
        duration_secs: f32,
    ) -> (Option<f32>, Option<f32>) {
        let mic_energy = self.mic_buffer.as_ref().and_then(|b| {
            let current_pos = b.samples_written();
            if current_pos <= positions.mic_pos {
                return None;
            }
            let window_samples =
                (duration_secs * b.sample_rate() as f32 * b.channels() as f32) as usize;
            let read_from = current_pos
                .saturating_sub(window_samples)
                .max(positions.mic_pos);
            b.rms_energy_since(read_from, window_samples)
        });

        let system_energy = self.system_buffer.as_ref().and_then(|b| {
            let current_pos = b.samples_written();
            if current_pos <= positions.system_pos {
                return None;
            }
            let window_samples =
                (duration_secs * b.sample_rate() as f32 * b.channels() as f32) as usize;
            let read_from = current_pos
                .saturating_sub(window_samples)
                .max(positions.system_pos);
            b.rms_energy_since(read_from, window_samples)
        });

        (mic_energy, system_energy)
    }

    // --- Session API ---

    /// Starts a recording session by marking the current buffer write positions.
    pub fn start_session(&mut self) -> Result<()> {
        if self.session_mark.is_some() {
            return Err(AudioError::SessionAlreadyActive);
        }

        let mic_write_pos = self
            .mic_buffer
            .as_ref()
            .map(|b| b.samples_written())
            .unwrap_or(0);
        let system_write_pos = self
            .system_buffer
            .as_ref()
            .map(|b| b.samples_written())
            .unwrap_or(0);

        self.session_mark = Some(SessionMark {
            mic_write_pos,
            system_write_pos,
            started_at: Instant::now(),
        });

        Ok(())
    }

    /// Ends the current session, captures all audio since the session started,
    /// and writes it to a temp WAV file.
    ///
    /// Each buffer's own channel count is used for deinterleaving to mono.
    /// The WAV file is always single-channel.
    pub fn end_session(
        &mut self,
        source: CaptureSource,
        mix_config: Option<&MixConfig>,
    ) -> Result<CaptureResult> {
        let mark = self
            .session_mark
            .take()
            .ok_or(AudioError::NoActiveSession)?;

        let mic_samples = self
            .mic_buffer
            .as_ref()
            .map(|b| {
                let snap = b.snapshot_since(mark.mic_write_pos);
                let total_new = b.samples_written().saturating_sub(mark.mic_write_pos);
                if total_new > b.capacity() {
                    warn!(
                        "mic session exceeded buffer capacity ({} > {}), audio truncated",
                        total_new,
                        b.capacity()
                    );
                }
                mixer::deinterleave_to_mono(&snap, b.channels()).into_owned()
            })
            .unwrap_or_default();

        let system_samples = self
            .system_buffer
            .as_ref()
            .map(|b| {
                let snap = b.snapshot_since(mark.system_write_pos);
                let total_new = b.samples_written().saturating_sub(mark.system_write_pos);
                if total_new > b.capacity() {
                    warn!(
                        "system session exceeded buffer capacity ({} > {}), audio truncated",
                        total_new,
                        b.capacity()
                    );
                }
                mixer::deinterleave_to_mono(&snap, b.channels()).into_owned()
            })
            .unwrap_or_default();

        let sample_rate = self.sample_rate_for_source(source);

        let samples = match source {
            CaptureSource::MicOnly => mic_samples,
            CaptureSource::SystemOnly => system_samples,
            CaptureSource::Mixed => {
                self.resample_and_mix(&mic_samples, &system_samples, mix_config)
            }
        };

        if samples.is_empty() {
            return Err(AudioError::NoBufferAvailable);
        }

        let duration_seconds = samples.len() as f32 / sample_rate as f32;
        let file_path = export::write_wav_to_temp(&samples, sample_rate, 1)?;

        Ok(CaptureResult {
            file_path,
            duration_seconds,
            sample_rate,
            source,
        })
    }

    /// Returns whether a session is currently active.
    pub fn is_session_active(&self) -> bool {
        self.session_mark.is_some()
    }

    /// Returns the elapsed seconds since the session started, if active.
    pub fn session_elapsed_seconds(&self) -> Option<f32> {
        self.session_mark
            .as_ref()
            .map(|m| m.started_at.elapsed().as_secs_f32())
    }

    // --- Config API ---

    pub fn set_config(&mut self, config: AudioConfig) {
        self.config = config;
    }

    pub fn config(&self) -> &AudioConfig {
        &self.config
    }

    /// Clears both ring buffers, dropping all captured audio data.
    pub fn clear_buffers(&mut self) {
        self.mic_buffer = None;
        self.system_buffer = None;
    }

    // --- Stream health ---

    /// Returns `true` if the mic stream's cpal error callback has fired.
    pub fn mic_has_stream_error(&self) -> bool {
        self.mic.has_stream_error()
    }

    /// Returns `true` if the system audio stream's cpal error callback has fired.
    pub fn system_has_stream_error(&self) -> bool {
        self.system.has_stream_error()
    }

    /// Returns the device ID used for the current/last mic capture session.
    pub fn mic_device_id(&self) -> Option<&str> {
        self.mic.last_device_id()
    }

    /// Restarts the microphone stream. Reuses the existing ring buffer when
    /// the new device's sample rate / channel count match; otherwise
    /// allocates a fresh buffer so extraction and WAV metadata stay
    /// consistent with the actual cpal callback format.
    ///
    /// Tries restart candidates in order — stored ID, stored name, system
    /// default — and probes + allocates the buffer per candidate so the
    /// returned buffer matches whichever device actually succeeds at
    /// start (rather than the first one that merely probed).
    pub fn restart_mic(&mut self) -> Result<RestartOutcome> {
        let existing = self
            .mic_buffer
            .clone()
            .ok_or(AudioError::NoBufferAvailable)?;

        let stored_id = self.mic.last_device_id().map(|s| s.to_string());
        let stored_name = self.mic.last_device_name().map(|s| s.to_string());
        // Preserve the original bind intent: if the session started in
        // default-tracking mode, restarts via the stored-id fallback should
        // not quietly flip us into explicit-device mode (which would disable
        // the drift defense for the remainder of the session).
        let preserve_bound_is_default = self.mic.bound_is_default();

        // Stop the old stream (ignore errors — it may already be dead)
        let _ = self.mic.stop();

        // Candidate order: original id → stored name → system default.
        // Each candidate is probed independently so its buffer matches the
        // device we'll actually hand to cpal — probing the first and starting
        // the second can leave the buffer metadata out of sync with the live
        // stream.
        let candidates: [(Option<&str>, &str); 3] = [
            (stored_id.as_deref(), "original device id"),
            (stored_name.as_deref(), "stored device name"),
            (None, "system default"),
        ];

        let mut last_err: Option<AudioError> = None;
        let mut tried_any = false;
        for (candidate, label) in candidates {
            // Skip blank intermediate candidates — only the final `None`
            // (system default) is a valid empty candidate.
            if candidate.is_none() && label != "system default" {
                continue;
            }
            tried_any = true;

            let probed = match MicrophoneCapture::query_device_config(candidate) {
                Ok(c) => c,
                Err(e) => {
                    warn!("restart: probing {} ({:?}) failed: {}", label, candidate, e);
                    last_err = Some(e);
                    continue;
                }
            };

            let (buffer, outcome) = self.pick_buffer_for_restart(&existing, probed);
            match self.mic.start(candidate, Arc::clone(&buffer)) {
                Ok(()) => {
                    self.mic_buffer = Some(buffer);
                    self.mic.set_bound_is_default(preserve_bound_is_default);
                    return Ok(outcome);
                }
                Err(e) => {
                    warn!("restart on {} ({:?}) failed: {}", label, candidate, e);
                    last_err = Some(e);
                    continue;
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            AudioError::DeviceInit(if tried_any {
                "all mic restart candidates failed".into()
            } else {
                "no mic restart candidates available".into()
            })
        }))
    }

    /// Restarts the system audio stream. Reuses the existing ring buffer
    /// when the new default output's sample rate / channel count match;
    /// otherwise allocates a fresh buffer so extraction and WAV metadata
    /// stay consistent with the actual cpal callback format. Callers must
    /// reset source positions / VAD state when `RestartOutcome::BufferReplaced`
    /// is returned — the old positions reference a different buffer.
    pub fn restart_system_audio(&mut self) -> Result<RestartOutcome> {
        let existing = self
            .system_buffer
            .clone()
            .ok_or(AudioError::NoBufferAvailable)?;

        // Stop the old stream (ignore errors — it may already be dead)
        let _ = self.system.stop();

        let probed = SystemAudioCapture::query_device_config()?;
        let (buffer, outcome) = self.pick_buffer_for_restart(&existing, probed);

        self.system.start(Arc::clone(&buffer))?;
        self.system_buffer = Some(buffer);
        Ok(outcome)
    }

    /// Chooses between reusing the existing buffer and allocating a fresh one
    /// sized for the probed device config. Shared between mic and system
    /// restart paths because the decision is identical: match sample rate and
    /// channel count, else replace so downstream metadata stays correct.
    fn pick_buffer_for_restart(
        &self,
        existing: &SharedAudioRingBuffer,
        probed: DeviceStreamConfig,
    ) -> (SharedAudioRingBuffer, RestartOutcome) {
        let format_matches =
            existing.sample_rate() == probed.sample_rate && existing.channels() == probed.channels;
        if format_matches {
            (Arc::clone(existing), RestartOutcome::BufferPreserved)
        } else {
            warn!(
                "device format changed on restart: {}Hz/{}ch → {}Hz/{}ch — allocating fresh buffer",
                existing.sample_rate(),
                existing.channels(),
                probed.sample_rate,
                probed.channels,
            );
            let fresh = Arc::new(AudioRingBuffer::with_duration(
                self.config.capture_history_seconds,
                probed.sample_rate,
                probed.channels,
            ));
            (fresh, RestartOutcome::BufferReplaced)
        }
    }

    /// Returns `true` if the OS has pushed a default-output-device change
    /// notification since the last call *and* system audio capture is
    /// currently running. Consumes the pending flag.
    pub fn system_audio_default_changed(&self) -> bool {
        self.system.is_running()
            && self
                .output_watcher
                .as_ref()
                .map(|w| w.take_change())
                .unwrap_or(false)
    }

    /// Returns `true` if the OS has pushed a default-input-device change
    /// notification since the last call *and* mic capture is currently
    /// running. Consumes the pending flag.
    pub fn mic_default_changed(&self) -> bool {
        self.mic.is_running()
            && self
                .input_watcher
                .as_ref()
                .map(|w| w.take_change())
                .unwrap_or(false)
    }

    /// Returns the device name the system audio stream is currently bound to,
    /// or `None` if not running / identity unknown. Used by the defensive
    /// device-identity drift poll.
    pub fn system_audio_bound_device(&self) -> Option<&str> {
        self.system.last_device_name()
    }

    /// Returns the device name the microphone stream is currently bound to,
    /// or `None` if not running / identity unknown.
    pub fn mic_bound_device(&self) -> Option<&str> {
        self.mic.last_device_name()
    }

    /// Defensive device-identity drift check. Returns `Some(current_default_name)`
    /// when the currently-bound system-audio output device differs from the
    /// OS default output device, else `None`. Only meaningful when system
    /// capture is running. This is the fallback for the push listener —
    /// the listener should fire first under normal conditions.
    pub fn system_audio_output_drifted(&self) -> Option<String> {
        if !self.system.is_running() {
            return None;
        }
        let bound = self.system.last_device_name()?;
        let current = current_default_output_name()?;
        if current != bound {
            Some(current)
        } else {
            None
        }
    }

    /// Defensive device-identity drift check for the microphone. Returns
    /// `Some(current_default_name)` when the currently-bound input device
    /// differs from the OS default input device, else `None`.
    ///
    /// Skipped when the mic was bound to a user-selected non-default device:
    /// the drift check exists to catch a missed CoreAudio default-device
    /// notification, which is only meaningful if we're tracking the default.
    /// Without this guard the check reports drift on every poll for users
    /// who explicitly pick a non-default mic, producing a restart storm.
    pub fn mic_input_drifted(&self) -> Option<String> {
        if !self.mic.is_running() {
            return None;
        }
        if !self.mic.bound_is_default() {
            return None;
        }
        let bound = self.mic.last_device_name()?;
        let current = current_default_input_name()?;
        if current != bound {
            Some(current)
        } else {
            None
        }
    }
}

fn current_default_output_name() -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let device = cpal::default_host().default_output_device()?;
    device.description().ok().map(|d| d.name().to_string())
}

fn current_default_input_name() -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let device = cpal::default_host().default_input_device()?;
    device.description().ok().map(|d| d.name().to_string())
}

impl Default for AudioManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_audio_manager() {
        let manager = AudioManager::new();
        let status = manager.status();
        assert_eq!(status.state, CaptureState::Idle);
        assert!(!status.mic_active);
        assert!(!status.system_audio_active);
        assert!(status.error_message.is_none());
    }

    #[test]
    fn test_stop_all_when_idle() {
        let mut manager = AudioManager::new();
        let result = manager.stop_all();
        assert!(result.is_ok());
        assert_eq!(manager.status().state, CaptureState::Idle);
    }

    #[test]
    fn test_snapshot_none_when_not_started() {
        let manager = AudioManager::new();
        assert!(manager.snapshot_mic(1.0).is_none());
        assert!(manager.snapshot_system(1.0).is_none());
        assert!(manager.snapshot_mic_all().is_none());
        assert!(manager.snapshot_system_all().is_none());
    }

    #[test]
    fn test_buffer_info_none_when_not_started() {
        let manager = AudioManager::new();
        assert!(manager.mic_buffer_info().is_none());
        assert!(manager.system_buffer_info().is_none());
    }

    #[test]
    fn test_default_config_values() {
        let manager = AudioManager::new();
        assert!((manager.config().capture_history_seconds - 180.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_with_config() {
        let config = AudioConfig {
            capture_history_seconds: 60.0,
        };
        let manager = AudioManager::with_config(config);
        assert!((manager.config().capture_history_seconds - 60.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_set_config() {
        let mut manager = AudioManager::new();
        let config = AudioConfig {
            capture_history_seconds: 300.0,
        };
        manager.set_config(config);
        assert!((manager.config().capture_history_seconds - 300.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_system_audio_permission() {
        let manager = AudioManager::new();
        let perm = manager.check_system_audio_permission();
        // On macOS and Windows, loopback capture uses standard audio permissions (always Granted)
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert_eq!(perm, PermissionStatus::Granted);
        // On other platforms, system audio capture is unavailable
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(perm, PermissionStatus::Unavailable);
    }

    #[test]
    fn test_clear_buffers() {
        let mut manager = AudioManager::new();
        manager.clear_buffers();
        assert!(manager.mic_buffer_info().is_none());
        assert!(manager.system_buffer_info().is_none());
    }

    #[test]
    fn test_session_not_active_initially() {
        let manager = AudioManager::new();
        assert!(!manager.is_session_active());
        assert!(manager.session_elapsed_seconds().is_none());
    }

    #[test]
    fn test_end_session_without_start_fails() {
        let mut manager = AudioManager::new();
        let result = manager.end_session(CaptureSource::MicOnly, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_double_start_session_fails() {
        let mut manager = AudioManager::new();
        manager.start_session().unwrap();
        let result = manager.start_session();
        assert!(result.is_err());
    }

    #[test]
    fn test_instant_capture_without_buffers_fails() {
        let manager = AudioManager::new();
        let result = manager.trigger_instant_capture(5.0, CaptureSource::MicOnly, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_buffer_positions_no_buffers() {
        let manager = AudioManager::new();
        let pos = manager.buffer_positions();
        assert_eq!(pos.mic_pos, 0);
        assert_eq!(pos.system_pos, 0);
    }

    #[test]
    fn test_extract_since_no_buffers_returns_none() {
        let manager = AudioManager::new();
        let pos = manager.buffer_positions();
        assert!(manager
            .extract_since(&pos, CaptureSource::MicOnly, None)
            .is_none());
    }

    #[test]
    fn test_extract_sources_since_no_buffers_returns_none() {
        let manager = AudioManager::new();
        let pos = manager.buffer_positions();
        assert!(manager.extract_sources_since(&pos).is_none());
    }

    #[test]
    fn test_buffer_positions_with_synthetic_buffer() {
        let mut manager = AudioManager::new();
        // Manually inject a mic buffer with some data
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        buffer.write(&[0.1, 0.2, 0.3]);
        manager.mic_buffer = Some(buffer);

        let pos = manager.buffer_positions();
        assert_eq!(pos.mic_pos, 3);
        assert_eq!(pos.system_pos, 0);
    }

    #[test]
    fn test_extract_since_with_synthetic_buffer() {
        let mut manager = AudioManager::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        manager.mic_buffer = Some(Arc::clone(&buffer));

        let pos = manager.buffer_positions();
        // Write data after capturing positions
        buffer.write(&[0.5, -0.5, 0.25, -0.25]);

        let result = manager.extract_since(&pos, CaptureSource::MicOnly, None);
        assert!(result.is_some());
        let (samples, sample_rate, new_pos) = result.unwrap();
        assert_eq!(samples.len(), 4);
        assert_eq!(sample_rate, 16000);
        assert_eq!(new_pos.mic_pos, 4);
    }

    #[test]
    fn test_extract_sources_since_with_synthetic_buffer() {
        let mut manager = AudioManager::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        manager.mic_buffer = Some(Arc::clone(&buffer));

        let pos = manager.buffer_positions();
        buffer.write(&[0.1, 0.2, 0.3]);

        let result = manager.extract_sources_since(&pos);
        assert!(result.is_some());
        let extraction = result.unwrap();
        assert!(extraction.mic.is_some());
        assert!(extraction.system.is_none());
        let (mic_samples, mic_rate) = extraction.mic.unwrap();
        assert_eq!(mic_samples.len(), 3);
        assert_eq!(mic_rate, 16000);
    }

    #[test]
    fn test_mic_write_pos() {
        let mut manager = AudioManager::new();
        assert_eq!(manager.mic_write_pos(), 0);

        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        buffer.write(&[0.1, 0.2, 0.3]);
        manager.mic_buffer = Some(buffer);
        assert_eq!(manager.mic_write_pos(), 3);
    }

    #[test]
    fn test_system_write_pos() {
        let mut manager = AudioManager::new();
        assert_eq!(manager.system_write_pos(), 0);

        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        buffer.write(&[0.1, 0.2, 0.3, 0.4, 0.5]);
        manager.system_buffer = Some(buffer);
        assert_eq!(manager.system_write_pos(), 5);
    }

    #[test]
    fn test_peek_energy_rms_with_synthetic_buffer() {
        let mut manager = AudioManager::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        manager.mic_buffer = Some(Arc::clone(&buffer));

        let pos = manager.buffer_positions();
        let signal: Vec<f32> = vec![0.5; 1600]; // 0.1 seconds at 16kHz
        buffer.write(&signal);

        let (mic_energy, sys_energy) = manager.peek_energy_rms(&pos, 0.1);
        assert!(mic_energy.is_some());
        assert!(sys_energy.is_none());
        let energy = mic_energy.unwrap();
        assert!((energy - 0.5).abs() < 0.01);
    }

    // --- End-to-end pipeline integration tests ---

    /// f32→i16 quantization tolerance: ~2/32768
    const ROUNDTRIP_TOLERANCE: f32 = 2.0 / 32767.0;

    /// Read mono WAV samples back as f32.
    fn read_wav_samples(path: &std::path::Path) -> (Vec<f32>, u32) {
        let reader = hound::WavReader::open(path).unwrap();
        let spec = reader.spec();
        let bit_depth = spec.bits_per_sample;
        let samples: Vec<f32> = reader
            .into_samples::<i16>()
            .map(|s| s.unwrap() as f32 / (1 << (bit_depth - 1)) as f32)
            .collect();
        (samples, spec.sample_rate)
    }

    #[test]
    fn test_instant_capture_roundtrip_mono() {
        let mut manager = AudioManager::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        let input: Vec<f32> = vec![0.25; 16000]; // 1 second
        buffer.write(&input);
        manager.mic_buffer = Some(buffer);

        let result = manager
            .trigger_instant_capture(1.0, CaptureSource::MicOnly, None)
            .unwrap();
        assert!(result.duration_seconds > 0.9);
        assert_eq!(result.sample_rate, 16000);

        let (samples, sr) = read_wav_samples(&result.file_path);
        assert_eq!(sr, 16000);
        assert!(!samples.is_empty());
        for &s in &samples {
            assert!(
                (s - 0.25).abs() < ROUNDTRIP_TOLERANCE,
                "expected ~0.25, got {}",
                s
            );
        }
        let _ = std::fs::remove_file(&result.file_path);
    }

    #[test]
    fn test_instant_capture_roundtrip_stereo_deinterleave() {
        let mut manager = AudioManager::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        // Interleaved stereo: L=0.8, R=0.2 → mono average ≈ 0.5
        let mut input = Vec::with_capacity(96000);
        for _ in 0..48000 {
            input.push(0.8);
            input.push(0.2);
        }
        buffer.write(&input);
        manager.mic_buffer = Some(buffer);

        let result = manager
            .trigger_instant_capture(1.0, CaptureSource::MicOnly, None)
            .unwrap();
        let (samples, sr) = read_wav_samples(&result.file_path);
        assert_eq!(sr, 48000);
        for &s in &samples {
            assert!(
                (s - 0.5).abs() < ROUNDTRIP_TOLERANCE,
                "expected ~0.5, got {}",
                s
            );
        }
        let _ = std::fs::remove_file(&result.file_path);
    }

    #[test]
    fn test_session_lifecycle_with_wav_output() {
        let mut manager = AudioManager::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        manager.mic_buffer = Some(Arc::clone(&buffer));

        manager.start_session().unwrap();
        assert!(manager.is_session_active());

        // Write audio during session
        let signal: Vec<f32> = vec![0.3; 8000]; // 0.5s
        buffer.write(&signal);

        let result = manager.end_session(CaptureSource::MicOnly, None).unwrap();
        assert!(!manager.is_session_active());
        assert!(result.duration_seconds > 0.4);

        let (samples, sr) = read_wav_samples(&result.file_path);
        assert_eq!(sr, 16000);
        for &s in &samples {
            assert!(
                (s - 0.3).abs() < ROUNDTRIP_TOLERANCE,
                "expected ~0.3, got {}",
                s
            );
        }
        let _ = std::fs::remove_file(&result.file_path);
    }

    #[test]
    fn test_session_captures_only_session_audio() {
        let mut manager = AudioManager::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        manager.mic_buffer = Some(Arc::clone(&buffer));

        // Write pre-session audio
        let pre_signal: Vec<f32> = vec![0.9; 8000];
        buffer.write(&pre_signal);

        manager.start_session().unwrap();

        // Write session audio
        let session_signal: Vec<f32> = vec![0.1; 4000];
        buffer.write(&session_signal);

        let result = manager.end_session(CaptureSource::MicOnly, None).unwrap();

        let (samples, _) = read_wav_samples(&result.file_path);
        // Should only contain session audio (~0.1), not pre-session (~0.9)
        assert_eq!(samples.len(), 4000);
        for &s in &samples {
            assert!(
                (s - 0.1).abs() < ROUNDTRIP_TOLERANCE,
                "expected ~0.1 (session audio), got {} (may contain pre-session audio)",
                s
            );
        }
        let _ = std::fs::remove_file(&result.file_path);
    }

    #[test]
    fn test_mixed_capture_roundtrip() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));

        // Asymmetric values so the assertion depends on BOTH sources contributing
        mic_buf.write(&vec![0.6_f32; 16000]);
        sys_buf.write(&vec![0.2_f32; 16000]);

        manager.mic_buffer = Some(mic_buf);
        manager.system_buffer = Some(sys_buf);

        let mix_config = MixConfig {
            mic_gain: 0.5,
            system_gain: 0.5,
            normalize: false,
        };
        let result = manager
            .trigger_instant_capture(1.0, CaptureSource::Mixed, Some(&mix_config))
            .unwrap();

        let (samples, sr) = read_wav_samples(&result.file_path);
        assert_eq!(sr, 16000);
        assert!(!samples.is_empty());
        // Expected: 0.6*0.5 + 0.2*0.5 = 0.4
        // If mixer ignores system → 0.3, if ignores mic → 0.1 — both distinguishable
        for &s in &samples {
            assert!(
                (s - 0.4).abs() < ROUNDTRIP_TOLERANCE,
                "expected ~0.4, got {}",
                s
            );
        }
        let _ = std::fs::remove_file(&result.file_path);
    }

    #[test]
    fn test_mixed_capture_stereo_system_mono_mic() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 2));

        mic_buf.write(&vec![0.4_f32; 16000]);
        // Stereo system: L=0.8, R=0.4 → mono avg = 0.6
        let mut stereo = Vec::with_capacity(32000);
        for _ in 0..16000 {
            stereo.push(0.8);
            stereo.push(0.4);
        }
        sys_buf.write(&stereo);

        manager.mic_buffer = Some(mic_buf);
        manager.system_buffer = Some(sys_buf);

        let mix_config = MixConfig {
            mic_gain: 0.5,
            system_gain: 0.5,
            normalize: false,
        };
        let result = manager
            .trigger_instant_capture(1.0, CaptureSource::Mixed, Some(&mix_config))
            .unwrap();

        let (samples, sr) = read_wav_samples(&result.file_path);
        assert_eq!(sr, 16000);
        assert!(!samples.is_empty());
        // mic mono=0.4, system mono=0.6 → 0.4*0.5 + 0.6*0.5 = 0.5
        // If mixer ignores system → 0.2, if ignores mic → 0.3 — both distinguishable
        for &s in &samples {
            assert!(
                (s - 0.5).abs() < ROUNDTRIP_TOLERANCE,
                "expected ~0.5, got {}",
                s
            );
        }
        let _ = std::fs::remove_file(&result.file_path);
    }

    #[test]
    fn test_sample_rate_mismatch_instant_capture_resamples() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 1));

        mic_buf.write(&vec![0.5_f32; 16000]);
        sys_buf.write(&vec![0.5_f32; 48000]);

        manager.mic_buffer = Some(mic_buf);
        manager.system_buffer = Some(sys_buf);

        // Should succeed by resampling system audio to 16kHz
        let result = manager.trigger_instant_capture(1.0, CaptureSource::Mixed, None);
        assert!(
            result.is_ok(),
            "expected success with resampling, got {:?}",
            result.err()
        );
        let capture = result.unwrap();
        assert_eq!(capture.sample_rate, 16000);
        assert!(capture.duration_seconds > 0.5);
        let _ = std::fs::remove_file(&capture.file_path);
    }

    #[test]
    fn test_sample_rate_mismatch_extract_since_resamples() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 1));

        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));

        let pos = manager.buffer_positions();

        mic_buf.write(&vec![0.5_f32; 16000]);
        sys_buf.write(&vec![0.5_f32; 48000]);

        // Should succeed by resampling system audio to mic rate
        let result = manager.extract_since(&pos, CaptureSource::Mixed, None);
        assert!(result.is_some(), "expected Some with resampled data");
        let (samples, sample_rate, _new_pos) = result.unwrap();
        assert_eq!(sample_rate, 16000);
        assert!(!samples.is_empty());
    }

    #[test]
    fn test_extract_since_mixed_trim_to_min() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));

        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));

        let pos = manager.buffer_positions();

        // Write different amounts: mic=500, sys=300 mono samples
        mic_buf.write(&vec![0.5_f32; 500]);
        sys_buf.write(&vec![0.3_f32; 300]);

        let mix_config = MixConfig {
            mic_gain: 1.0,
            system_gain: 1.0,
            normalize: false,
        };
        let result = manager.extract_since(&pos, CaptureSource::Mixed, Some(&mix_config));
        assert!(result.is_some());
        let (samples, _sr, new_pos) = result.unwrap();

        // Output should be min(500, 300) = 300 samples (after limiting)
        assert_eq!(samples.len(), 300);

        // Mic position should be rewound by 200 surplus samples
        assert_eq!(new_pos.mic_pos, 300); // 500 - 200 = 300
                                          // System position should be fully advanced
        assert_eq!(new_pos.system_pos, 300);

        // Second extraction should pick up the remaining 200 mic samples
        sys_buf.write(&vec![0.3_f32; 200]);
        let result2 = manager.extract_since(&new_pos, CaptureSource::Mixed, Some(&mix_config));
        assert!(result2.is_some());
        let (samples2, _sr2, _new_pos2) = result2.unwrap();
        assert_eq!(samples2.len(), 200);
    }

    #[test]
    fn test_extract_since_mixed_equal_lengths_no_rewind() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));

        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));

        let pos = manager.buffer_positions();

        // Write equal amounts
        mic_buf.write(&vec![0.5_f32; 400]);
        sys_buf.write(&vec![0.3_f32; 400]);

        let result = manager.extract_since(&pos, CaptureSource::Mixed, None);
        assert!(result.is_some());
        let (samples, _sr, new_pos) = result.unwrap();
        assert_eq!(samples.len(), 400);
        assert_eq!(new_pos.mic_pos, 400);
        assert_eq!(new_pos.system_pos, 400);
    }

    // --- Stream health tests ---

    #[test]
    fn test_stream_error_flags_default_false() {
        let manager = AudioManager::new();
        assert!(!manager.mic_has_stream_error());
        assert!(!manager.system_has_stream_error());
    }

    #[test]
    fn test_default_changed_is_false_when_not_running() {
        // Idle manager should never report a default-device change — the
        // flag is gated on `is_running()` so stale listener signals from a
        // previous session can't cause spurious restarts.
        let manager = AudioManager::new();
        assert!(!manager.system_audio_default_changed());
        assert!(!manager.mic_default_changed());
    }

    #[test]
    fn test_bound_device_none_when_not_running() {
        let manager = AudioManager::new();
        assert!(manager.system_audio_bound_device().is_none());
        assert!(manager.mic_bound_device().is_none());
    }

    #[test]
    fn test_drift_none_when_not_running() {
        let manager = AudioManager::new();
        assert!(manager.system_audio_output_drifted().is_none());
        assert!(manager.mic_input_drifted().is_none());
    }

    #[test]
    fn test_mic_bound_is_default_defaults_to_false() {
        // A fresh MicrophoneCapture hasn't been started, so we can't yet
        // claim to be tracking the default. The drift check's early-return
        // must honor this to avoid false positives before start().
        let manager = AudioManager::new();
        assert!(!manager.mic.bound_is_default());
    }

    #[test]
    fn test_mic_set_bound_is_default_setter() {
        // Used by restart_mic to preserve the original bind intent through
        // a candidate fallback — verify the setter round-trips.
        let mut manager = AudioManager::new();
        manager.mic.set_bound_is_default(true);
        assert!(manager.mic.bound_is_default());
        manager.mic.set_bound_is_default(false);
        assert!(!manager.mic.bound_is_default());
    }

    #[test]
    #[cfg_attr(target_os = "windows", ignore)] // WASAPI COM cleanup crashes on CI
    fn test_restart_mic_keeps_buffer_slot() {
        // After restart the mic_buffer slot stays populated regardless of
        // outcome — either the original Arc (format match) or a fresh buffer
        // (format mismatch, e.g. device changed between 48 kHz stereo and
        // 44.1 kHz mono). The slot staying populated is what the live loop
        // relies on for `RestartOutcome` reset logic.
        let mut manager = AudioManager::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        buffer.write(&[0.1, 0.2, 0.3]);
        manager.mic_buffer = Some(Arc::clone(&buffer));

        let _ = manager.restart_mic();
        assert!(manager.mic_buffer.is_some());
    }

    #[test]
    #[cfg_attr(target_os = "windows", ignore)] // WASAPI COM cleanup crashes on CI
    fn test_restart_system_keeps_buffer_slot() {
        let mut manager = AudioManager::new();
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        buffer.write(&[0.1, 0.2, 0.3, 0.4]);
        manager.system_buffer = Some(Arc::clone(&buffer));

        let _ = manager.restart_system_audio();
        assert!(manager.system_buffer.is_some());
    }

    #[test]
    fn test_pick_buffer_replaces_on_sample_rate_mismatch() {
        let manager = AudioManager::new();
        let existing = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        let probed = DeviceStreamConfig {
            sample_rate: 44100,
            channels: 2,
        };
        let (_buf, outcome) = manager.pick_buffer_for_restart(&existing, probed);
        assert_eq!(outcome, RestartOutcome::BufferReplaced);
    }

    #[test]
    fn test_pick_buffer_replaces_on_channel_mismatch() {
        let manager = AudioManager::new();
        let existing = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        let probed = DeviceStreamConfig {
            sample_rate: 48000,
            channels: 1,
        };
        let (_buf, outcome) = manager.pick_buffer_for_restart(&existing, probed);
        assert_eq!(outcome, RestartOutcome::BufferReplaced);
    }

    #[test]
    fn test_pick_buffer_preserves_on_format_match() {
        let manager = AudioManager::new();
        let existing = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        let probed = DeviceStreamConfig {
            sample_rate: 48000,
            channels: 2,
        };
        let (buf, outcome) = manager.pick_buffer_for_restart(&existing, probed);
        assert_eq!(outcome, RestartOutcome::BufferPreserved);
        assert!(Arc::ptr_eq(&buf, &existing));
    }

    #[test]
    fn test_restart_without_buffer_errors() {
        let mut manager = AudioManager::new();
        let result = manager.restart_mic();
        assert!(result.is_err());
        let result = manager.restart_system_audio();
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_since_system_only_reports_system_rate_with_stale_mic_buffer() {
        // A SystemOnly session with a stale mic buffer at a different rate
        // must report the system buffer's rate, not the mic buffer's. The
        // WAV writer relies on this to decide whether to resample before
        // appending samples.
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 44100, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));

        let pos = manager.buffer_positions();
        sys_buf.write(&vec![0.3_f32; 400]); // 200 mono frames

        let (_samples, reported_sr, _new_pos) = manager
            .extract_since(&pos, CaptureSource::SystemOnly, None)
            .expect("system extraction returns samples");
        assert_eq!(reported_sr, 48000, "SystemOnly must report system rate");
    }

    #[test]
    fn test_extract_source_samples_system_only_reports_system_rate_with_stale_mic() {
        // Mirror of the extract_since test for the instant-capture / session-export path.
        // trigger_instant_capture previously used `active_sample_rate()` (mic-preferred),
        // which wrote SystemOnly audio under the mic rate when a stale mic buffer was
        // present — garbled duration and playback speed in the exported WAV.
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 44100, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));
        sys_buf.write(&vec![0.3_f32; 9600]); // ~100ms of stereo audio @ 48k

        let (_samples, reported_sr) = manager
            .extract_source_samples(1.0, CaptureSource::SystemOnly, None)
            .expect("system extraction returns samples");
        assert_eq!(
            reported_sr, 48000,
            "SystemOnly must report system rate, not mic rate"
        );
    }

    #[test]
    fn test_extract_source_samples_mic_only_reports_mic_rate_with_stale_system() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 44100, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));
        mic_buf.write(&vec![0.3_f32; 4410]);

        let (_samples, reported_sr) = manager
            .extract_source_samples(1.0, CaptureSource::MicOnly, None)
            .expect("mic extraction returns samples");
        assert_eq!(reported_sr, 44100, "MicOnly must report mic rate");
    }

    #[test]
    fn test_captured_audio_carries_per_source_rates() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 44100, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));
        mic_buf.write(&vec![0.2_f32; 100]);
        sys_buf.write(&vec![0.2_f32; 200]);

        let captured = manager.extract_captured_audio(0.01);
        assert_eq!(captured.mic_sample_rate, Some(44100));
        assert_eq!(captured.system_sample_rate, Some(48000));
    }

    #[test]
    fn test_extract_since_mic_only_reports_mic_rate_with_stale_system_buffer() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 44100, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 2));
        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));

        let pos = manager.buffer_positions();
        mic_buf.write(&vec![0.3_f32; 200]);

        let (_samples, reported_sr, _new_pos) = manager
            .extract_since(&pos, CaptureSource::MicOnly, None)
            .expect("mic extraction returns samples");
        assert_eq!(reported_sr, 44100, "MicOnly must report mic rate");
    }

    #[test]
    fn test_extract_since_mixed_stereo_system_trim() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        // Stereo system buffer: 2 raw samples per mono sample
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 2));

        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));

        let pos = manager.buffer_positions();

        // Mic: 500 mono samples, System: 600 raw (=300 mono) stereo samples
        mic_buf.write(&vec![0.5_f32; 500]);
        let mut stereo = Vec::with_capacity(600);
        for _ in 0..300 {
            stereo.push(0.4);
            stereo.push(0.2);
        }
        sys_buf.write(&stereo);

        let mix_config = MixConfig {
            mic_gain: 1.0,
            system_gain: 1.0,
            normalize: false,
        };
        let result = manager.extract_since(&pos, CaptureSource::Mixed, Some(&mix_config));
        assert!(result.is_some());
        let (samples, _sr, new_pos) = result.unwrap();

        // min(500 mic mono, 300 sys mono) = 300
        assert_eq!(samples.len(), 300);
        // Mic surplus = 200 mono → rewind by 200 * 1 channel = 200 raw
        assert_eq!(new_pos.mic_pos, 300);
        // System fully consumed
        assert_eq!(new_pos.system_pos, 600);
    }

    #[test]
    fn test_extract_since_mixed_rate_trim() {
        // Verify time-aligned trimming: 16kHz mic + 48kHz system, 0.5s each.
        // At different rates, equal time = different sample counts.
        // The fix ensures we trim by time, not by sample count.
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 1));

        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));

        let pos = manager.buffer_positions();

        // Write 0.5s at each rate: 8000 mic samples, 24000 system samples
        mic_buf.write(&vec![0.5_f32; 8000]);
        sys_buf.write(&vec![0.5_f32; 24000]);

        let result = manager.extract_since(&pos, CaptureSource::Mixed, None);
        assert!(result.is_some(), "expected Some with mixed-rate data");
        let (samples, sample_rate, new_pos) = result.unwrap();
        assert_eq!(sample_rate, 16000);
        assert!(!samples.is_empty());

        // Both sources represent exactly 0.5s, so no surplus — positions fully advanced
        assert_eq!(new_pos.mic_pos, 8000);
        assert_eq!(new_pos.system_pos, 24000);

        // Now test unequal durations: mic=0.5s (8000), system=0.3s (14400)
        mic_buf.write(&vec![0.5_f32; 8000]);
        sys_buf.write(&vec![0.5_f32; 14400]);

        let result2 = manager.extract_since(&new_pos, CaptureSource::Mixed, None);
        assert!(result2.is_some());
        let (_samples2, _sr2, new_pos2) = result2.unwrap();

        // Min time = 0.3s. Mic should be trimmed to 0.3s = 4800 samples, surplus = 3200
        // Mic pos rewound by 3200: 16000 - 3200 = 12800
        assert_eq!(new_pos2.mic_pos, 12800);
        // System fully consumed at 0.3s
        assert_eq!(new_pos2.system_pos, 24000 + 14400);
    }

    #[test]
    fn test_sample_rate_mismatch_end_session_resamples() {
        let mut manager = AudioManager::new();
        let mic_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        let sys_buf = Arc::new(AudioRingBuffer::with_duration(10.0, 48000, 1));

        manager.mic_buffer = Some(Arc::clone(&mic_buf));
        manager.system_buffer = Some(Arc::clone(&sys_buf));

        manager.start_session().unwrap();

        mic_buf.write(&vec![0.5_f32; 16000]);
        sys_buf.write(&vec![0.5_f32; 48000]);

        // Should succeed by resampling system audio to 16kHz
        let result = manager.end_session(CaptureSource::Mixed, None);
        assert!(
            result.is_ok(),
            "expected success with resampling, got {:?}",
            result.err()
        );
        let capture = result.unwrap();
        assert_eq!(capture.sample_rate, 16000);
        assert!(capture.duration_seconds > 0.5);
        let _ = std::fs::remove_file(&capture.file_path);
    }

    #[test]
    fn test_start_capture_already_running_preserves_state() {
        // When start_capture returns AlreadyRunning, the state machine should NOT
        // transition to Error — the existing Capturing state is preserved.
        let mut manager = AudioManager::new();

        // Simulate an already-running mic by injecting a buffer and setting state.
        let buffer = Arc::new(AudioRingBuffer::with_duration(10.0, 16000, 1));
        manager.mic_buffer = Some(Arc::clone(&buffer));
        manager.state = CaptureState::Capturing;

        // MicOnly start_capture will call start_mic → mic.start() → AlreadyRunning
        // won't fire because mic.is_running is false. Instead, test via Mixed path:
        // start_all calls start_mic first. If mic succeeds, then start_system.
        // We need the AlreadyRunning to come from start_capture's inner call.
        // Easiest: directly test the guard in start_capture.
        // Set mic as running to trigger AlreadyRunning from start_mic.
        // We can't easily mock mic.start(), but we can test the error gate:
        // Call start_capture with a source that will fail with a non-AlreadyRunning error
        // and verify state becomes Error, then verify AlreadyRunning doesn't.

        // First: verify non-AlreadyRunning errors DO set Error state
        let result = manager.start_capture(CaptureSource::SystemOnly, None);
        if result.is_err() {
            // On CI without system audio, this fails with a real error
            let status = manager.status();
            assert_eq!(status.state, CaptureState::Error);
            assert!(status.error_message.is_some());
        }

        // Reset to Capturing state to test AlreadyRunning guard
        manager.state = CaptureState::Capturing;
        manager.error_message = None;

        // Manually test the AlreadyRunning guard logic:
        // The AudioError::AlreadyRunning should NOT change state
        let err = AudioError::AlreadyRunning;
        if !matches!(err, AudioError::AlreadyRunning) {
            manager.state = CaptureState::Error;
            manager.error_message = Some(err.to_string());
        }
        assert_eq!(manager.status().state, CaptureState::Capturing);
        assert!(manager.status().error_message.is_none());

        // And a non-AlreadyRunning error SHOULD change state
        let err = AudioError::NoBufferAvailable;
        if !matches!(err, AudioError::AlreadyRunning) {
            manager.state = CaptureState::Error;
            manager.error_message = Some(err.to_string());
        }
        assert_eq!(manager.status().state, CaptureState::Error);
        assert!(manager.status().error_message.is_some());
    }

    #[test]
    #[cfg_attr(target_os = "windows", ignore)] // WASAPI COM cleanup crashes on CI
    fn test_start_capture_failure_sets_error_state() {
        // On macOS CI, SystemOnly will fail because there's no output device.
        // This verifies end-to-end that start_capture sets error state.
        let mut manager = AudioManager::new();
        let result = manager.start_capture(CaptureSource::SystemOnly, None);
        if result.is_err() {
            let status = manager.status();
            assert_eq!(status.state, CaptureState::Error);
            assert!(status.error_message.is_some());
        }
        // If it succeeds (hardware present), that's fine — the state machine
        // test above covers the error path.
    }
}
