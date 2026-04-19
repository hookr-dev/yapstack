use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, error, info, warn};
use yapstack_audio::manager::AudioManager;
use yapstack_audio::BufferPositions;
use yapstack_common::types::CaptureSource;

use super::error::{validate_session_id, CommandError};

use super::audio::{AudioManagerState, CaptureSourceDto, MixConfigDto};
use super::transcription::{TranscriptSegmentDto, WhisperClientState};

// --- DTOs ---

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub enum AudioSourceLabel {
    Mic,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LiveTranscriptionConfig {
    /// RMS energy threshold below which audio is considered silence. Default: 0.01.
    pub silence_threshold: f32,
    /// Milliseconds of continuous silence before triggering a chunk. Default: 800.
    pub silence_duration_ms: u32,
    /// Force a chunk after this many seconds of continuous speech. Default: 30.
    pub max_chunk_seconds: f32,
    /// Seconds of buffer lookback at session start (backfill). Default: 0.
    pub backfill_seconds: f32,
    /// Audio source.
    pub source: CaptureSourceDto,
    /// Mix config for Mixed source (kept for backward compat).
    pub mix_config: Option<MixConfigDto>,
    /// Whisper language.
    pub language: Option<String>,
    /// Max characters of prior transcript to feed as Whisper prompt context. Default: 350.
    pub prompt_context_chars: Option<u32>,
    /// Seconds of all-source silence before clearing prompt context to prevent
    /// hallucination from stale context. Default: 5.0. Set to 0 to disable.
    pub prompt_decay_silence_seconds: Option<f32>,
    /// Session ID for streaming WAV recording. If set, audio is incrementally
    /// written to `$APP_DATA_DIR/audio/{session_id}.wav` during the session.
    pub session_id: Option<String>,
    /// Custom directory for saving WAV files. If None, uses `$APP_DATA_DIR/audio/`.
    pub audio_save_location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub enum LiveTranscriptionPhase {
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct LiveTranscriptionStatus {
    pub phase: LiveTranscriptionPhase,
    pub chunks_processed: u32,
    pub total_audio_seconds: f32,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub effective_start_epoch_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct LiveTranscriptionStartResult {
    pub effective_start_epoch_ms: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct LiveSegmentEvent {
    pub chunk_index: u32,
    /// Which audio source produced this chunk.
    pub source: AudioSourceLabel,
    /// Segments from this chunk.
    pub segments: Vec<TranscriptSegmentDto>,
    /// Offset in seconds from the start of live transcription.
    pub audio_offset_seconds: f32,
    /// Duration of this chunk's audio in seconds.
    pub chunk_duration_seconds: f32,
    /// Per-source accumulated text so far.
    pub accumulated_text: String,
    /// Whether this chunk came from backfill processing (true) or live VAD (false).
    pub is_backfill: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SessionWavReadyEvent {
    pub session_id: String,
    pub file_path: String,
    pub duration_seconds: f32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SessionWavErrorEvent {
    pub session_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SessionWavWarningEvent {
    pub session_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct LiveTranscriptionWarningEvent {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct StreamHealthEvent {
    pub source: AudioSourceLabel,
    /// "restarted" | "restart_failed" | "restart_abandoned"
    pub status: String,
    pub message: String,
}

/// Internal state for streaming WAV recording during a live session.
struct SessionWavState {
    writer: yapstack_audio::SessionWavWriter,
    flush_positions: BufferPositions,
    source: CaptureSource,
    mix_config: Option<yapstack_audio::MixConfig>,
    session_id: String,
    flush_count: u32,
}

// --- Shared context ---

/// Immutable shared context for transcription operations.
///
/// During live transcription, the WhisperClient is extracted from the shared
/// `WhisperClientState` and held privately in `whisper_client`. This eliminates
/// mutex contention with other commands that may access `WhisperClientState`
/// (e.g. `transcribe_audio`, `shutdown_whisper_client`). The client is returned
/// to shared state when the live transcription loop ends.
#[derive(Clone)]
struct TranscriptionContext {
    /// Private client for the live transcription loop — zero contention.
    whisper_client: Arc<Mutex<Option<yapstack_transcription::WhisperClient>>>,
    /// Reference to the shared state so we can return the client when done.
    shared_whisper_state: WhisperClientState,
    app_handle: AppHandle,
    config: LiveTranscriptionConfig,
    /// Shared prompt context bridging backfill → live transcription.
    bridged_prompt: Arc<Mutex<String>>,
}

/// Result of transcribing a single chunk.
struct ChunkResult {
    event: LiveSegmentEvent,
    chunk_duration: f32,
}

/// Outcome of a transcription attempt — determines cursor management and loop control.
enum TranscribeOutcome {
    /// Transcription succeeded with segments.
    Success(ChunkResult),
    /// Transcription failed non-fatally (chunk skipped, audio can be retried).
    Skipped,
    /// Sidecar is dead and could not be restarted — loop should exit.
    SidecarDead,
}

/// RAII guard that deletes a temp file on drop (including panic unwind).
struct TempFileGuard(std::path::PathBuf);
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Read-only input data for a single transcription chunk.
struct ChunkInput<'a> {
    samples: &'a [f32],
    sample_rate: u32,
    audio_offset_seconds: f32,
    source_label: AudioSourceLabel,
    is_backfill: bool,
}

/// Session-level mutable accumulators passed through transcription functions.
struct SessionAccumulators {
    shared_prompt: String,
    total_chunks: u32,
    total_audio_seconds: f32,
    /// When the last successful transcription occurred. Used for prompt decay —
    /// if no transcription has happened for `prompt_decay_silence_seconds`, all
    /// prompt context is cleared to prevent stale text from causing hallucinations.
    last_transcription_at: Option<Instant>,
}

/// A single chunk of backfill audio with its sample rate.
struct BackfillChunk<'a> {
    samples: &'a [f32],
    sample_rate: u32,
}

// --- Controller ---

pub struct LiveTranscriptionController {
    task_handle: tokio::task::JoinHandle<()>,
    stop_tx: Option<oneshot::Sender<()>>,
    session_id: Option<String>,
    effective_start_epoch_ms: f64,
}

impl LiveTranscriptionController {
    pub fn is_running(&self) -> bool {
        !self.task_handle.is_finished()
    }
}

pub type LiveTranscriptionState = Arc<Mutex<Option<LiveTranscriptionController>>>;

// --- VAD helpers ---

/// Per-source voice activity detection state.
struct SourceVadState {
    /// Which audio source this tracks.
    label: AudioSourceLabel,
    is_speaking: bool,
    /// Scalar buffer position when speech started (extraction start point).
    speech_start_pos: usize,
    /// Scalar read cursor — advances as we extract audio.
    cursor: usize,
    /// When speech started (for max-duration enforcement).
    speech_start_time: Option<Instant>,
    /// When energy last dropped below threshold (start of silence).
    silence_since: Option<Instant>,
    /// Per-source chunk counter.
    chunk_index: u32,
    /// Per-source accumulated transcription text.
    accumulated_text: String,
    /// Buffer position corresponding to T=0 on the session timeline (rewound backfill position).
    session_start_pos: usize,
    /// Sample rate of this source's ring buffer.
    source_sample_rate: u32,
    /// Channel count of this source's ring buffer (interleaved).
    source_channels: u16,
    /// Consecutive failures from the same speech_start_pos (for retry cap).
    consecutive_failures: u32,
    /// Last observed write_pos for stall detection.
    last_seen_write_pos: usize,
    /// When last_seen_write_pos last advanced.
    last_write_pos_advance: Instant,
    /// Number of restart attempts for this source.
    restart_attempts: u32,
    /// When the last restart was attempted (for cooldown).
    last_restart_at: Option<Instant>,
}

impl SourceVadState {
    fn new(
        label: AudioSourceLabel,
        initial_pos: usize,
        session_start_pos: usize,
        sample_rate: u32,
        channels: u16,
    ) -> Self {
        Self {
            label,
            is_speaking: false,
            speech_start_pos: initial_pos,
            cursor: initial_pos,
            speech_start_time: None,
            silence_since: None,
            chunk_index: 0,
            accumulated_text: String::new(),
            session_start_pos,
            source_sample_rate: sample_rate,
            source_channels: channels,
            consecutive_failures: 0,
            last_seen_write_pos: initial_pos,
            last_write_pos_advance: Instant::now(),
            restart_attempts: 0,
            last_restart_at: None,
        }
    }
}

/// Describes what action the VAD state machine wants to take.
enum VadAction {
    /// No action needed — continue polling.
    None,
    /// Speech pause detected — chunk and transcribe.
    Chunk,
    /// Max duration exceeded while still speaking — force chunk.
    ForceChunk,
}

/// Polls the VAD state machine for a single source.
/// Manages `speech_start_time` internally on state transitions.
fn poll_vad(
    state: &mut SourceVadState,
    energy: Option<f32>,
    silence_threshold: f32,
    silence_duration: Duration,
    max_chunk_duration: Duration,
) -> VadAction {
    let Some(energy) = energy else {
        return VadAction::None;
    };

    let is_loud = energy >= silence_threshold;

    if state.is_speaking {
        if is_loud {
            state.silence_since = None;

            let speech_elapsed = state
                .speech_start_time
                .map(|t| t.elapsed())
                .unwrap_or_default();
            if speech_elapsed >= max_chunk_duration {
                return VadAction::ForceChunk;
            }

            VadAction::None
        } else {
            let silence_start = *state.silence_since.get_or_insert_with(Instant::now);
            if silence_start.elapsed() >= silence_duration {
                VadAction::Chunk
            } else {
                VadAction::None
            }
        }
    } else {
        if is_loud {
            state.is_speaking = true;
            state.silence_since = None;
            state.speech_start_time = Some(Instant::now());
        }
        VadAction::None
    }
}

// --- Helpers ---

fn emit_status(
    app_handle: &AppHandle,
    phase: LiveTranscriptionPhase,
    chunks: u32,
    audio_secs: f32,
) {
    let _ = app_handle.emit(
        "live-transcription-status",
        LiveTranscriptionStatus {
            phase,
            chunks_processed: chunks,
            total_audio_seconds: audio_secs,
            error_message: None,
            session_id: None,
            effective_start_epoch_ms: None,
        },
    );
}

/// Reconstruct `BufferPositions` from the source vec for `peek_energy_rms`.
fn build_cursor(sources: &[SourceVadState]) -> BufferPositions {
    let mut pos = BufferPositions::default();
    for s in sources {
        match s.label {
            AudioSourceLabel::Mic => pos.mic_pos = s.cursor,
            AudioSourceLabel::System => pos.system_pos = s.cursor,
        }
    }
    pos
}

/// Get the current write position for a source from the manager.
fn source_write_pos(manager: &AudioManager, label: &AudioSourceLabel) -> usize {
    match label {
        AudioSourceLabel::Mic => manager.mic_write_pos(),
        AudioSourceLabel::System => manager.system_write_pos(),
    }
}

/// Extract mono audio from a source's ring buffer starting at `from_pos`.
///
/// Returns `(audio, new_pos)` where `new_pos` is the atomic write position
/// captured at the same instant as the snapshot. This eliminates the race
/// window where cpal callbacks advance write_pos between snapshot and
/// position query.
fn extract_source_audio(
    manager: &AudioManager,
    label: &AudioSourceLabel,
    from_pos: usize,
) -> (Option<(Vec<f32>, u32)>, usize) {
    let buf = match label {
        AudioSourceLabel::Mic => manager.mic_buffer(),
        AudioSourceLabel::System => manager.system_buffer(),
    };
    let Some(buf) = buf else {
        return (None, from_pos);
    };
    let (snap, new_pos) = buf.snapshot_since_with_pos(from_pos);
    if snap.is_empty() {
        return (None, new_pos);
    }
    let mono = yapstack_common::audio::deinterleave_to_mono(&snap, buf.channels()).into_owned();
    (Some((mono, buf.sample_rate())), new_pos)
}

/// Process a single source's chunk: extract audio, transcribe, emit events, reset state.
/// Returns `true` if a fatal error occurred (sidecar dead) and the loop should exit.
async fn process_source_chunk(
    vad: &mut SourceVadState,
    audio_state: &AudioManagerState,
    ctx: &TranscriptionContext,
    is_force_chunk: bool,
    session: &mut SessionAccumulators,
) -> bool {
    // Single atomic read: extract audio AND capture write position together
    let (extraction, new_pos) = {
        let manager = audio_state.lock().await;
        extract_source_audio(&manager, &vad.label, vad.speech_start_pos)
    };

    let mut outcome = TranscribeOutcome::Skipped;

    if let Some((samples, sample_rate)) = extraction {
        let chunk_duration = samples.len() as f32 / sample_rate as f32;
        if chunk_duration >= MIN_CHUNK_DURATION_SECS {
            // Deterministic offset from buffer position delta. No wall-clock drift.
            let samples_since_start = vad.speech_start_pos.saturating_sub(vad.session_start_pos);
            let audio_offset = samples_since_start as f32
                / (vad.source_sample_rate as f32 * vad.source_channels as f32);

            debug!(
                "live chunk: source={:?} offset={:.2}s duration={:.2}s samples={} (pos: speech={} session_start={})",
                vad.label,
                audio_offset,
                chunk_duration,
                samples.len(),
                vad.speech_start_pos,
                vad.session_start_pos,
            );

            let input = ChunkInput {
                samples: &samples,
                sample_rate,
                audio_offset_seconds: audio_offset,
                source_label: vad.label,
                is_backfill: false,
            };
            outcome = transcribe_and_emit_chunk(
                ctx,
                &input,
                &mut vad.chunk_index,
                &mut vad.accumulated_text,
                session,
            )
            .await;
        }
    }

    // Manage cursor and VAD state based on outcome
    match outcome {
        TranscribeOutcome::Success(_) => {
            vad.consecutive_failures = 0;
            vad.cursor = new_pos;
            if is_force_chunk {
                vad.speech_start_pos = new_pos;
                vad.silence_since = None;
                vad.speech_start_time = Some(Instant::now());
            } else {
                vad.is_speaking = false;
                vad.silence_since = None;
                vad.speech_start_pos = new_pos;
                vad.speech_start_time = None;
            }
            false
        }
        TranscribeOutcome::Skipped => {
            vad.consecutive_failures += 1;
            // Advance cursor but keep speech_start_pos for retry — the failed audio
            // region will be included in the next chunk extraction.
            vad.cursor = new_pos;
            vad.is_speaking = false;
            vad.silence_since = None;
            vad.speech_start_time = None;

            // Retry cap: after MAX_CHUNK_RETRIES failures from the same speech_start_pos,
            // abandon the chunk to prevent ever-growing extractions.
            if vad.consecutive_failures >= MAX_CHUNK_RETRIES {
                warn!(
                    "live transcription: {} consecutive failures for source {:?} — abandoning chunk",
                    vad.consecutive_failures, vad.label
                );
                vad.speech_start_pos = new_pos;
                vad.consecutive_failures = 0;
                let _ = ctx.app_handle.emit(
                    "live-transcription-warning",
                    LiveTranscriptionWarningEvent {
                        message: "Some audio could not be transcribed and was skipped".into(),
                    },
                );
            }
            false
        }
        TranscribeOutcome::SidecarDead => true,
    }
}

/// Advance the idle cursor to the current write position when not speaking.
async fn advance_idle_cursor(vad: &mut SourceVadState, audio_state: &AudioManagerState) {
    let new_pos = {
        let manager = audio_state.lock().await;
        source_write_pos(&manager, &vad.label)
    };
    vad.cursor = new_pos;
    vad.speech_start_pos = new_pos;
}

/// Trim leading silence from mono audio. Returns (trimmed_slice, offset_seconds).
/// Scans in small windows; keeps a pad before first detected energy to avoid
/// clipping speech onset. Returns an empty slice if the chunk is entirely silent.
fn trim_leading_silence(
    samples: &[f32],
    sample_rate: u32,
    silence_threshold: f32,
    pad_seconds: f32,
) -> (&[f32], f32) {
    let window_samples = (sample_rate as f32 * 0.05) as usize; // 50 ms windows
    if window_samples == 0 || samples.is_empty() {
        return (samples, 0.0);
    }

    let mut first_loud = None;
    for (i, window) in samples.chunks(window_samples).enumerate() {
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
        if rms > silence_threshold {
            first_loud = Some(i * window_samples);
            break;
        }
    }

    let Some(loud_pos) = first_loud else {
        return (&[], 0.0); // all silent — return empty so caller can skip
    };

    let pad_samples = (pad_seconds * sample_rate as f32) as usize;
    let trim_start = loud_pos.saturating_sub(pad_samples);
    let trim_offset = trim_start as f32 / sample_rate as f32;
    (&samples[trim_start..], trim_offset)
}

// --- Prompt decay ---

/// Clear prompt context when no successful transcription has occurred within
/// `decay_seconds`. This prevents stale text from causing hallucinations
/// (e.g. saying "Hello", waiting 10s with ambient noise, then Whisper
/// hallucinating "Hello" from noise because the old prompt is still primed).
///
/// Uses time-since-last-transcription rather than VAD silence detection,
/// because ambient noise (keyboard, AC, fan) easily exceeds the RMS threshold
/// and keeps resetting the silence timer, preventing decay from ever firing.
/// Noise-only chunks produce `TranscribeOutcome::Skipped` and do NOT update
/// `last_transcription_at`, so intermittent noise cannot prevent decay.
///
/// Returns `true` if any prompt state was cleared.
fn check_prompt_decay(
    sources: &mut [SourceVadState],
    shared_prompt: &mut String,
    decay_seconds: f32,
    last_transcription_at: Option<Instant>,
) -> bool {
    if decay_seconds <= 0.0 {
        return false;
    }
    let Some(last_t) = last_transcription_at else {
        return false;
    };
    if last_t.elapsed() < Duration::from_secs_f32(decay_seconds) {
        return false;
    }
    let has_prompt_state =
        !shared_prompt.is_empty() || sources.iter().any(|s| !s.accumulated_text.is_empty());
    if !has_prompt_state {
        return false;
    }
    shared_prompt.clear();
    for source in sources.iter_mut() {
        source.accumulated_text.clear();
    }
    true
}

// --- Background loop ---

/// Internal poll interval for the VAD loop.
const POLL_INTERVAL_MS: u64 = 300;
/// Minimum chunk duration in seconds below which a chunk is skipped.
const MIN_CHUNK_DURATION_SECS: f32 = 0.1;
/// Number of consecutive empty WAV flush extractions before emitting an error event.
const WAV_FLUSH_ERROR_THRESHOLD: u32 = 10;
/// Interval (in consecutive empty flushes) for emitting periodic warnings.
const WAV_FLUSH_WARNING_INTERVAL: u32 = 20;
/// Interval (in successful flushes) for emitting periodic diagnostic logs.
const WAV_FLUSH_DIAGNOSTIC_INTERVAL: u32 = 100;
/// Maximum chars of accumulated transcription text kept per source.
const MAX_ACCUMULATED_TEXT_CHARS: usize = 1000;
/// Max consecutive failures from the same speech_start_pos before abandoning the chunk.
const MAX_CHUNK_RETRIES: u32 = 3;
/// Seconds of write_pos stall before triggering a stream restart.
const STREAM_STALL_THRESHOLD_SECS: f32 = 2.0;
/// Maximum auto-restart attempts per source before giving up.
const STREAM_RESTART_MAX_ATTEMPTS: u32 = 3;
/// Minimum seconds between restart attempts for the same source.
const STREAM_RESTART_COOLDOWN_SECS: f32 = 5.0;

// --- Pure stream health decision helpers ---

/// Returns `true` if a write-position stall should trigger a stream restart for the
/// given source. On Windows, system audio loopback produces zero samples when nothing
/// is playing — this is normal WASAPI behavior, not a stream failure.
fn should_stall_restart(label: &AudioSourceLabel) -> bool {
    if cfg!(target_os = "windows") && matches!(label, AudioSourceLabel::System) {
        return false;
    }
    true
}

/// Window used by the pre-flight health check to observe whether `write_pos`
/// is advancing. Must be long enough to see at least one cpal callback
/// (typical buffer periods are 5–20 ms) and short enough to not add noticeable
/// latency to the hotkey → dictation-start flow.
const PREFLIGHT_SAMPLE_WINDOW_MS: u64 = 80;

/// Pre-flight stream health check. Run before the live-transcription loop
/// starts to catch silently-stalled cpal streams — device changes, Bluetooth
/// disconnects, OS wake-from-sleep — that don't raise an error callback. The
/// in-loop watchdog only fires after `STREAM_STALL_THRESHOLD_SECS` (2 s), by
/// which point the first transcribed chunk has already been extracted from
/// stale buffer data, producing wildly inaccurate or empty transcriptions
/// after long idle periods.
///
/// Strategy: snapshot each relevant source's `write_pos`, wait briefly, check
/// again. If any source has its error flag set or failed to advance, restart
/// it before spawning the live loop. A failed restart on a user-requested
/// source propagates as an error; a failed system-audio restart in mixed mode
/// is logged but allowed to proceed (the loop will degrade to mic-only).
async fn preflight_stream_health(
    audio_state: &AudioManagerState,
    source: &CaptureSource,
    app_handle: &AppHandle,
) -> Result<(), CommandError> {
    let check_mic = matches!(source, CaptureSource::MicOnly | CaptureSource::Mixed);
    let check_system = matches!(source, CaptureSource::SystemOnly | CaptureSource::Mixed);

    if !check_mic && !check_system {
        return Ok(());
    }

    let (mic_initial, sys_initial, mic_err, sys_err) = {
        let m = audio_state.lock().await;
        (
            m.mic_write_pos(),
            m.system_write_pos(),
            m.mic_has_stream_error(),
            m.system_has_stream_error(),
        )
    };

    tokio::time::sleep(Duration::from_millis(PREFLIGHT_SAMPLE_WINDOW_MS)).await;

    let mut manager = audio_state.lock().await;
    let mut restarted: Vec<AudioSourceLabel> = Vec::new();

    if check_mic {
        let mic_now = manager.mic_write_pos();
        let stalled = mic_now == mic_initial && should_stall_restart(&AudioSourceLabel::Mic);
        if mic_err || stalled {
            warn!(
                "preflight: mic stream needs restart (error={}, stalled={})",
                mic_err, stalled
            );
            match manager.restart_mic() {
                Ok(()) => restarted.push(AudioSourceLabel::Mic),
                Err(e) => {
                    error!("preflight: mic restart failed: {}", e);
                    return Err(CommandError::from(e));
                }
            }
        }
    }

    if check_system {
        let sys_now = manager.system_write_pos();
        let stalled = sys_now == sys_initial && should_stall_restart(&AudioSourceLabel::System);
        if sys_err || stalled {
            warn!(
                "preflight: system stream needs restart (error={}, stalled={})",
                sys_err, stalled
            );
            match manager.restart_system_audio() {
                Ok(()) => restarted.push(AudioSourceLabel::System),
                Err(e) => {
                    error!("preflight: system restart failed: {}", e);
                    if matches!(source, CaptureSource::SystemOnly) {
                        return Err(CommandError::from(e));
                    }
                    // Mixed mode: degrade to mic-only, surface a health event.
                    let _ = app_handle.emit(
                        "stream-health",
                        StreamHealthEvent {
                            source: AudioSourceLabel::System,
                            status: "restart_failed".into(),
                            message: format!("preflight system restart failed: {e}"),
                        },
                    );
                }
            }
        }
    }

    drop(manager);

    for label in restarted {
        let name = source_display_name(&label);
        let _ = app_handle.emit(
            "stream-health",
            StreamHealthEvent {
                source: label,
                status: "restarted".into(),
                message: format!("{name} stream restarted (preflight)"),
            },
        );
    }

    Ok(())
}

/// Classification of consecutive empty WAV flush extractions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyFlushClassification {
    /// Normal Windows WASAPI silence — no system audio playing.
    WindowsSilence,
    /// Real extraction failure — possible stream error or sample-rate mismatch.
    Error,
}

/// Classifies consecutive empty WAV flush extractions. On Windows, system-only sessions
/// produce zero samples when nothing is playing (WASAPI event-driven loopback). This is
/// normal and should not be reported as an error unless the cpal error flag is set.
fn classify_empty_flush(source: CaptureSource, has_stream_error: bool) -> EmptyFlushClassification {
    let is_windows_system_silence = cfg!(target_os = "windows")
        && matches!(source, CaptureSource::SystemOnly)
        && !has_stream_error;
    if is_windows_system_silence {
        EmptyFlushClassification::WindowsSilence
    } else {
        EmptyFlushClassification::Error
    }
}

/// Chunk audio into segments of approximately `chunk_size` samples, but refine
/// boundaries by scanning backward from each split point to find a silence gap.
/// This avoids splitting mid-word. Falls back to the fixed boundary if no silence
/// gap is found within the last `search_window` samples.
fn chunk_at_silence_boundaries(
    samples: &[f32],
    chunk_size: usize,
    sample_rate: u32,
    silence_threshold: f32,
) -> Vec<&[f32]> {
    if samples.is_empty() || chunk_size == 0 {
        return vec![];
    }

    // Search backward up to 5 seconds from each boundary
    let search_window = (5.0 * sample_rate as f32) as usize;
    // Silence gap must be at least 100ms to be considered a natural pause
    let min_silence_samples = (0.1 * sample_rate as f32) as usize;
    // Scan in 10ms windows for RMS
    let rms_window = (0.01 * sample_rate as f32) as usize;

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < samples.len() {
        let nominal_end = (start + chunk_size).min(samples.len());

        // If this is the last chunk or it's short enough, just take the rest
        if nominal_end >= samples.len() || samples.len() - start <= chunk_size {
            chunks.push(&samples[start..samples.len()]);
            break;
        }

        // Scan backward from nominal_end looking for a silence gap
        let scan_start = nominal_end.saturating_sub(search_window);
        let mut best_split = None;
        let mut silence_run = 0usize;

        // Scan backward in rms_window steps
        let mut pos = nominal_end;
        while pos > scan_start + rms_window {
            pos -= rms_window;
            let window_end = (pos + rms_window).min(samples.len());
            let window = &samples[pos..window_end];
            let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();

            if rms < silence_threshold {
                silence_run += rms_window;
                if silence_run >= min_silence_samples {
                    // Found a silence gap — split at the start of this silence region
                    best_split = Some(pos + silence_run);
                    break;
                }
            } else {
                silence_run = 0;
            }
        }

        let split_at = best_split.unwrap_or(nominal_end);
        chunks.push(&samples[start..split_at]);
        start = split_at;
    }

    chunks
}

/// Process backfill audio concurrently with the live VAD loop.
/// Chunks audio by max_chunk_seconds with soft boundaries at silence gaps,
/// trims leading silence, interleaves across sources.
/// Emits segments with offsets 0..backfill_seconds, then emits a `backfill-complete` event.
async fn process_backfill(
    ctx: TranscriptionContext,
    backfill_audio: Vec<(Vec<f32>, u32, AudioSourceLabel)>,
    backfill_done: Arc<AtomicBool>,
) {
    info!("backfill: starting concurrent processing");

    // Build per-source chunk lists with soft boundaries at silence gaps
    let mut source_entries: Vec<(AudioSourceLabel, Vec<BackfillChunk<'_>>)> = Vec::new();
    for (samples, sample_rate, label) in &backfill_audio {
        let chunk_size = (ctx.config.max_chunk_seconds * *sample_rate as f32) as usize;
        let chunks: Vec<BackfillChunk<'_>> = chunk_at_silence_boundaries(
            samples,
            chunk_size.max(1),
            *sample_rate,
            ctx.config.silence_threshold,
        )
        .into_iter()
        .map(|c| BackfillChunk {
            samples: c,
            sample_rate: *sample_rate,
        })
        .collect();
        source_entries.push((*label, chunks));
    }

    // Per-source state
    let mut chunk_indices: Vec<u32> = vec![0; source_entries.len()];
    let mut accumulated_texts: Vec<String> = vec![String::new(); source_entries.len()];
    let mut offsets: Vec<f32> = vec![0.0; source_entries.len()];
    let mut session = SessionAccumulators {
        shared_prompt: String::new(),
        total_chunks: 0,
        total_audio_seconds: 0.0,
        last_transcription_at: None,
    };

    // Interleave: process window 0 for all sources, then window 1, etc.
    let total_windows = source_entries
        .iter()
        .map(|(_, chunks)| chunks.len())
        .max()
        .unwrap_or(0);

    for window_idx in 0..total_windows {
        for (source_idx, (label, chunks)) in source_entries.iter().enumerate() {
            if let Some(chunk) = chunks.get(window_idx) {
                // Trim leading silence so Whisper gets audio starting near speech
                let (trimmed, trim_offset) = trim_leading_silence(
                    chunk.samples,
                    chunk.sample_rate,
                    ctx.config.silence_threshold,
                    0.2, // 200ms pad before detected speech
                );
                let adjusted_offset = offsets[source_idx] + trim_offset;

                // Skip entirely silent chunks — nothing for Whisper to transcribe
                if trimmed.is_empty() {
                    debug!(
                        "backfill chunk: source={:?} window={} skipped (silent)",
                        label, window_idx
                    );
                    offsets[source_idx] += chunk.samples.len() as f32 / chunk.sample_rate as f32;
                    continue;
                }

                debug!(
                    "backfill chunk: source={:?} window={} offset={:.2}s trim_offset={:.3}s trimmed_samples={}",
                    label, window_idx, adjusted_offset, trim_offset, trimmed.len()
                );

                let input = ChunkInput {
                    samples: trimmed,
                    sample_rate: chunk.sample_rate,
                    audio_offset_seconds: adjusted_offset,
                    source_label: *label,
                    is_backfill: true,
                };
                transcribe_and_emit_chunk(
                    &ctx,
                    &input,
                    &mut chunk_indices[source_idx],
                    &mut accumulated_texts[source_idx],
                    &mut session,
                )
                .await;

                // Advance by FULL original chunk duration (not trimmed)
                offsets[source_idx] += chunk.samples.len() as f32 / chunk.sample_rate as f32;
            }
        }
    }

    info!(
        "backfill: completed {} chunks, {:.1}s audio",
        session.total_chunks, session.total_audio_seconds
    );

    // Bridge prompt context to live loop (Change 9: move instead of clone)
    {
        let mut prompt = ctx.bridged_prompt.lock().await;
        let max_prompt = ctx.config.prompt_context_chars.unwrap_or(350) as usize;
        if session.shared_prompt.len() > max_prompt {
            let boundary = session
                .shared_prompt
                .ceil_char_boundary(session.shared_prompt.len() - max_prompt);
            *prompt = session.shared_prompt[boundary..].to_string();
        } else {
            *prompt = std::mem::take(&mut session.shared_prompt);
        }
    }

    // Signal that backfill is done — live loop can now count empty flushes toward error threshold
    backfill_done.store(true, Ordering::Release);

    // Signal frontend that backfill processing is done
    let _ = ctx.app_handle.emit("backfill-complete", ());
}

/// Returns a human-readable name for a source label.
fn source_display_name(label: &AudioSourceLabel) -> &'static str {
    match label {
        AudioSourceLabel::Mic => "Microphone",
        AudioSourceLabel::System => "System audio",
    }
}

/// Checks stream health for all sources and triggers restarts if needed.
/// Returns `true` if any source was restarted (caller may want to skip VAD this tick).
async fn check_stream_health(
    sources: &mut [SourceVadState],
    audio_state: &AudioManagerState,
    app_handle: &AppHandle,
) {
    for source in sources.iter_mut() {
        if source.restart_attempts >= STREAM_RESTART_MAX_ATTEMPTS {
            continue; // Already exhausted retries for this source
        }

        // Cooldown: skip if we recently attempted a restart for this source
        if let Some(last) = source.last_restart_at {
            if last.elapsed() < Duration::from_secs_f32(STREAM_RESTART_COOLDOWN_SECS) {
                continue;
            }
        }

        let mut needs_restart = false;
        let mut reason = "";

        {
            let manager = audio_state.lock().await;

            // Layer 1: cpal error callback flag (instant detection)
            let has_error = match source.label {
                AudioSourceLabel::Mic => manager.mic_has_stream_error(),
                AudioSourceLabel::System => manager.system_has_stream_error(),
            };
            if has_error {
                needs_restart = true;
                reason = "stream error callback fired";
            }

            // Layer 2: write_pos stall detection (~2s latency)
            // On Windows, system audio loopback produces zero samples when nothing is
            // playing — this is normal behavior, not a stream failure. Only use stall
            // detection for mic streams on Windows; system streams rely solely on the
            // cpal error flag (Layer 1).
            if !needs_restart {
                let current_pos = source_write_pos(&manager, &source.label);
                if current_pos > source.last_seen_write_pos {
                    source.last_seen_write_pos = current_pos;
                    source.last_write_pos_advance = Instant::now();
                } else if source.last_write_pos_advance.elapsed()
                    > Duration::from_secs_f32(STREAM_STALL_THRESHOLD_SECS)
                    && should_stall_restart(&source.label)
                {
                    needs_restart = true;
                    reason = "write position stalled";
                }
            }
        }

        if !needs_restart {
            continue;
        }

        let source_name = source_display_name(&source.label);
        warn!(
            "stream health: {} needs restart ({}), attempt {}/{}",
            source_name,
            reason,
            source.restart_attempts + 1,
            STREAM_RESTART_MAX_ATTEMPTS
        );

        source.last_restart_at = Some(Instant::now());

        let restart_result = {
            let mut manager = audio_state.lock().await;
            match source.label {
                AudioSourceLabel::Mic => manager.restart_mic(),
                AudioSourceLabel::System => manager.restart_system_audio(),
            }
        };

        match restart_result {
            Ok(()) => {
                info!("stream health: {} restarted successfully", source_name);
                source.restart_attempts = 0;
                source.last_write_pos_advance = Instant::now();
                let _ = app_handle.emit(
                    "stream-health",
                    StreamHealthEvent {
                        source: source.label,
                        status: "restarted".into(),
                        message: format!("{} stream restarted ({})", source_name, reason),
                    },
                );
            }
            Err(e) => {
                source.restart_attempts += 1;
                error!(
                    "stream health: {} restart failed (attempt {}): {}",
                    source_name, source.restart_attempts, e
                );
                let status = if source.restart_attempts >= STREAM_RESTART_MAX_ATTEMPTS {
                    "restart_abandoned"
                } else {
                    "restart_failed"
                };
                let _ = app_handle.emit(
                    "stream-health",
                    StreamHealthEvent {
                        source: source.label,
                        status: status.into(),
                        message: format!(
                            "{} stream restart failed: {} (attempt {}/{})",
                            source_name, e, source.restart_attempts, STREAM_RESTART_MAX_ATTEMPTS
                        ),
                    },
                );
            }
        }
    }
}

async fn live_transcription_loop(
    audio_state: AudioManagerState,
    ctx: TranscriptionContext,
    mut stop_rx: oneshot::Receiver<()>,
    mut session_wav_state: Option<SessionWavState>,
) {
    let source = ctx.config.source.clone().into();
    let check_mic = matches!(source, CaptureSource::MicOnly | CaptureSource::Mixed);
    let check_system = matches!(source, CaptureSource::SystemOnly | CaptureSource::Mixed);

    let silence_threshold = ctx.config.silence_threshold;
    let silence_duration = Duration::from_millis(ctx.config.silence_duration_ms as u64);
    let max_chunk_duration = Duration::from_secs_f32(ctx.config.max_chunk_seconds);

    // Get initial cursor positions and build source list
    let (mut sources, backfill_audio) = {
        let manager = audio_state.lock().await;
        let positions = manager.buffer_positions();

        let has_backfill = ctx.config.backfill_seconds > 0.0;
        let mut sources: Vec<SourceVadState> = Vec::with_capacity(2);
        let mut backfill: Vec<(Vec<f32>, u32, AudioSourceLabel)> = Vec::new();

        if check_mic {
            let (initial_pos, session_start, sr, ch) = if let Some(buf) = manager.mic_buffer() {
                let sr = buf.sample_rate();
                let ch = buf.channels();
                if has_backfill {
                    let raw = (ctx.config.backfill_seconds * sr as f32 * ch as f32) as usize;
                    // Round down to frame boundary for correct deinterleaving
                    let rewind = raw - (raw % ch as usize);
                    let rewound = positions.mic_pos.saturating_sub(rewind);
                    (rewound, rewound, sr, ch)
                } else {
                    (positions.mic_pos, positions.mic_pos, sr, ch)
                }
            } else {
                (positions.mic_pos, positions.mic_pos, 48000, 1)
            };
            sources.push(SourceVadState::new(
                AudioSourceLabel::Mic,
                initial_pos,
                session_start,
                sr,
                ch,
            ));
        }
        if check_system {
            if let Some(buf) = manager.system_buffer() {
                let sr = buf.sample_rate();
                let ch = buf.channels();
                let (initial_pos, session_start) = if has_backfill {
                    let raw = (ctx.config.backfill_seconds * sr as f32 * ch as f32) as usize;
                    // Round down to frame boundary for correct deinterleaving
                    let rewind = raw - (raw % ch as usize);
                    let rewound = positions.system_pos.saturating_sub(rewind);
                    (rewound, rewound)
                } else {
                    (positions.system_pos, positions.system_pos)
                };
                sources.push(SourceVadState::new(
                    AudioSourceLabel::System,
                    initial_pos,
                    session_start,
                    sr,
                    ch,
                ));
            } else if matches!(source, CaptureSource::Mixed) {
                warn!("mixed mode: system buffer unavailable, running mic-only");
            }
        }

        // Extract backfill audio and reset cursors to current positions
        if has_backfill {
            info!(
                "live transcription: extracted backfill audio ({:.1}s) for concurrent processing",
                ctx.config.backfill_seconds
            );
            for s in sources.iter_mut() {
                let (audio, current) = extract_source_audio(&manager, &s.label, s.cursor);
                if let Some((samples, sr)) = audio {
                    info!(
                        "backfill extract: source={:?} cursor={} write_pos={} samples={} sr={} duration={:.2}s",
                        s.label, s.cursor, current, samples.len(), sr, samples.len() as f32 / sr as f32
                    );
                    backfill.push((samples, sr, s.label));
                } else {
                    warn!(
                        "backfill: no audio available for source={:?} (cursor={}, write_pos={})",
                        s.label, s.cursor, current
                    );
                }
                // Reset cursor to current write position — VAD loop starts fresh
                s.cursor = current;
                s.speech_start_pos = current;
            }
        }

        (sources, backfill)
    };

    // Spawn concurrent backfill processing
    let backfill_done = Arc::new(AtomicBool::new(false));
    let backfill_handle = if !backfill_audio.is_empty() {
        // Namespace live chunk indices to avoid collision with backfill (0..N)
        for s in &mut sources {
            s.chunk_index = 10_000;
        }
        let backfill_ctx = ctx.clone();
        let backfill_done_clone = backfill_done.clone();
        let handle = tokio::spawn(process_backfill(
            backfill_ctx,
            backfill_audio,
            backfill_done_clone,
        ));
        let abort_handle = handle.abort_handle();
        Some((handle, abort_handle))
    } else {
        backfill_done.store(true, Ordering::Release);
        None
    };

    let mut session = SessionAccumulators {
        shared_prompt: String::new(),
        total_chunks: 0,
        total_audio_seconds: 0.0,
        last_transcription_at: None,
    };
    let mut wav_flush_none_count: u32 = 0;
    let mut prompt_seeded_from_backfill = false;

    emit_status(&ctx.app_handle, LiveTranscriptionPhase::Running, 0, 0.0);

    let poll_interval = tokio::time::Duration::from_millis(POLL_INTERVAL_MS);
    let poll_energy_secs = POLL_INTERVAL_MS as f32 / 1000.0;
    let mut exited_fatal = false;

    loop {
        let should_stop = tokio::select! {
            _ = tokio::time::sleep(poll_interval) => false,
            _ = &mut stop_rx => true,
        };

        // Single lock: energy check + WAV flush extraction
        let (mic_energy, system_energy, wav_flush_data) = {
            let manager = audio_state.lock().await;
            let energies = manager.peek_energy_rms(&build_cursor(&sources), poll_energy_secs);
            let flush = session_wav_state.as_ref().and_then(|ws| {
                manager.extract_since(&ws.flush_positions, ws.source, ws.mix_config.as_ref())
            });
            (energies.0, energies.1, flush)
        };

        // Write WAV data outside the lock
        if let Some((samples, _sr, new_pos)) = wav_flush_data {
            wav_flush_none_count = 0;
            if let Some(ref mut ws) = session_wav_state {
                if let Err(e) = ws.writer.write_samples(&samples) {
                    error!(
                        "session WAV write error ({} samples may be lost): {}",
                        samples.len(),
                        e
                    );
                }
                // Always advance — retrying partial writes would duplicate samples
                ws.flush_positions = new_pos;
                // Periodic diagnostic (~every 30s at 300ms intervals)
                ws.flush_count += 1;
                if ws.flush_count % WAV_FLUSH_DIAGNOSTIC_INTERVAL == 0 {
                    debug!(
                        "session WAV progress: flushes={}, samples_written={}, duration={:.1}s",
                        ws.flush_count,
                        ws.writer.samples_written(),
                        ws.writer.duration_seconds()
                    );
                }
            }
        } else if session_wav_state.is_some() {
            wav_flush_none_count += 1;
            debug!(
                "session WAV flush: no data (consecutive: {})",
                wav_flush_none_count
            );
            // Only count toward the error threshold after backfill completes.
            // During backfill, empty flushes are expected because backfill is
            // consuming ring buffer reads for the first few seconds.
            if wav_flush_none_count == WAV_FLUSH_ERROR_THRESHOLD {
                if backfill_done.load(Ordering::Acquire) {
                    if let Some(ref ws) = session_wav_state {
                        let has_stream_error = {
                            let mgr = audio_state.lock().await;
                            mgr.system_has_stream_error()
                        };
                        match classify_empty_flush(ws.source, has_stream_error) {
                            EmptyFlushClassification::WindowsSilence => {
                                info!(
                                    "session WAV: empty extractions for SystemOnly session {} on Windows — emitting warning (no stream error)",
                                    ws.session_id
                                );
                                let _ = ctx.app_handle.emit(
                                    "session-wav-warning",
                                    SessionWavWarningEvent {
                                        session_id: ws.session_id.clone(),
                                        message:
                                            "No system audio detected — recording will resume when audio plays"
                                                .to_string(),
                                    },
                                );
                            }
                            EmptyFlushClassification::Error => {
                                warn!(
                                    "session WAV: 10 consecutive empty extractions for session {} — emitting error event",
                                    ws.session_id
                                );
                                let _ = ctx.app_handle.emit(
                                    "session-wav-error",
                                    SessionWavErrorEvent {
                                        session_id: ws.session_id.clone(),
                                        message:
                                            "No audio data available for recording — audio may not be saved"
                                                .to_string(),
                                    },
                                );
                            }
                        }
                    }
                } else {
                    debug!("session WAV flush: resetting empty count (backfill still running)");
                    wav_flush_none_count = 0;
                }
            }
            if wav_flush_none_count.is_multiple_of(WAV_FLUSH_WARNING_INTERVAL) {
                let silence_secs = wav_flush_none_count as f32 * POLL_INTERVAL_MS as f32 / 1000.0;
                warn!(
                    "session WAV flush: {} consecutive empty extractions ({:.1}s) — possible sample rate mismatch or no audio data",
                    wav_flush_none_count, silence_secs
                );
            }
        }

        // Stream health watchdog: check for cpal error flags and write_pos stalls.
        // Triggers auto-restart if a stream has died silently.
        // restart_mic() tries the previously stored device first, then falls back to
        // the provided name (None = system default).
        check_stream_health(&mut sources, &audio_state, &ctx.app_handle).await;

        // Seed live prompt from backfill context once available.
        // Must seed both shared_prompt AND each source's accumulated_text,
        // since transcribe_chunk() uses accumulated_text as the Whisper prompt.
        if !prompt_seeded_from_backfill {
            let bridged = ctx.bridged_prompt.lock().await;
            if !bridged.is_empty() {
                if session.shared_prompt.is_empty() {
                    session.shared_prompt = bridged.clone();
                }
                for source in &mut sources {
                    if source.accumulated_text.is_empty() {
                        source.accumulated_text = bridged.clone();
                    }
                }
                prompt_seeded_from_backfill = true;
                session.last_transcription_at = Some(Instant::now());
                debug!(
                    "live loop: seeded prompt from backfill ({} chars)",
                    bridged.len()
                );
            }
        }

        let mut fatal = false;
        for source in &mut sources {
            let energy = match source.label {
                AudioSourceLabel::Mic => mic_energy,
                AudioSourceLabel::System => system_energy,
            };

            let mut action = poll_vad(
                source,
                energy,
                silence_threshold,
                silence_duration,
                max_chunk_duration,
            );

            if should_stop && source.is_speaking {
                action = VadAction::Chunk;
            }

            match action {
                VadAction::Chunk | VadAction::ForceChunk => {
                    let is_fatal = process_source_chunk(
                        source,
                        &audio_state,
                        &ctx,
                        matches!(action, VadAction::ForceChunk),
                        &mut session,
                    )
                    .await;
                    if is_fatal {
                        fatal = true;
                        break;
                    }
                }
                VadAction::None if !source.is_speaking => {
                    advance_idle_cursor(source, &audio_state).await;
                }
                _ => {}
            }
        }

        if fatal {
            error!("live transcription: sidecar died and could not be restarted — stopping");
            let _ = ctx.app_handle.emit(
                "live-transcription-status",
                LiveTranscriptionStatus {
                    phase: LiveTranscriptionPhase::Error,
                    chunks_processed: session.total_chunks,
                    total_audio_seconds: session.total_audio_seconds,
                    error_message: Some(
                        "Transcription engine stopped unexpectedly and could not be restarted"
                            .to_string(),
                    ),
                    session_id: ctx.config.session_id.clone(),
                    effective_start_epoch_ms: None,
                },
            );
            exited_fatal = true;
            break;
        }

        // Prompt decay: clear shared_prompt + accumulated_text when no transcription
        // has occurred for prompt_decay_silence_seconds (default 5s)
        let prompt_decay_secs = ctx.config.prompt_decay_silence_seconds.unwrap_or(5.0);
        if check_prompt_decay(
            &mut sources,
            &mut session.shared_prompt,
            prompt_decay_secs,
            session.last_transcription_at,
        ) {
            info!(
                "prompt decay: cleared shared_prompt ({:.1}s since last transcription)",
                session
                    .last_transcription_at
                    .map(|t| t.elapsed().as_secs_f32())
                    .unwrap_or(0.0)
            );
            session.last_transcription_at = None;
        }

        if should_stop {
            break;
        }
    }

    // Final WAV flush: extract any remaining audio and finalize
    if let Some(mut ws) = session_wav_state {
        // Extract remaining audio since last flush
        let final_flush = {
            let manager = audio_state.lock().await;
            manager.extract_since(&ws.flush_positions, ws.source, ws.mix_config.as_ref())
        };
        if let Some((samples, _sr, _new_pos)) = final_flush {
            if let Err(e) = ws.writer.write_samples(&samples) {
                error!("session WAV final flush write failed: {}", e);
            }
        }

        if ws.writer.samples_written() == 0 {
            // No audio was ever written — delete the empty WAV file
            warn!(
                "session WAV had 0 samples written — deleting empty file for session {}",
                ws.session_id
            );
            let wav_path = ws.writer.path().to_path_buf();
            // Finalize to release file handle, then delete
            let _ = ws.writer.finalize();
            let _ = std::fs::remove_file(&wav_path);
            let _ = ctx.app_handle.emit(
                "session-wav-error",
                SessionWavErrorEvent {
                    session_id: ws.session_id,
                    message: "No audio was recorded — WAV file not saved".to_string(),
                },
            );
        } else {
            match ws.writer.finalize() {
                Ok((path, duration)) => {
                    info!(
                        "session WAV finalized: {} ({:.1}s)",
                        path.display(),
                        duration
                    );
                    let _ = ctx.app_handle.emit(
                        "session-wav-ready",
                        SessionWavReadyEvent {
                            session_id: ws.session_id,
                            file_path: path.to_string_lossy().to_string(),
                            duration_seconds: duration,
                        },
                    );
                }
                Err(e) => {
                    error!("session WAV finalize failed: {}", e);
                }
            }
        }
    }

    // Wait for concurrent backfill to finish before emitting Stopped (30s timeout)
    if let Some((handle, abort_handle)) = backfill_handle {
        match tokio::time::timeout(Duration::from_secs(30), handle).await {
            Ok(_) => {}
            Err(_) => {
                warn!("backfill task did not complete within 30s — aborting");
                abort_handle.abort();
            }
        }
    }

    // Only emit Stopped if we didn't already emit Error (avoids duplicate finalization)
    if !exited_fatal {
        emit_status(
            &ctx.app_handle,
            LiveTranscriptionPhase::Stopped,
            session.total_chunks,
            session.total_audio_seconds,
        );
    }

    info!(
        "live transcription stopped: {} chunks, {:.1}s total audio",
        session.total_chunks, session.total_audio_seconds
    );
}

/// Transcribes a chunk of audio and returns a `TranscribeOutcome`.
///
/// - `Success`: transcription produced segments
/// - `Skipped`: non-fatal failure (temp file error, empty chunk, transient sidecar error)
/// - `SidecarDead`: sidecar died and could not be restarted — caller should exit the loop
async fn transcribe_chunk(
    ctx: &TranscriptionContext,
    input: &ChunkInput<'_>,
    chunk_index: &mut u32,
    accumulated_text: &mut String,
    session: &mut SessionAccumulators,
) -> TranscribeOutcome {
    let chunk_duration = input.samples.len() as f32 / input.sample_rate as f32;
    if chunk_duration < MIN_CHUNK_DURATION_SECS {
        return TranscribeOutcome::Skipped;
    }

    // Write temp WAV — guard ensures cleanup on all exit paths including panics.
    let _wav_guard;
    let wav_path =
        match yapstack_audio::export::write_wav_to_temp(input.samples, input.sample_rate, 1) {
            Ok(p) => {
                _wav_guard = TempFileGuard(p.clone());
                p
            }
            Err(e) => {
                error!("live transcription: failed to write WAV: {}", e);
                return TranscribeOutcome::Skipped;
            }
        };

    // Transcribe using the private (zero-contention) client.
    // Use per-source accumulated_text as initial_prompt to avoid cross-source
    // contamination in Mixed mode (e.g. system audio biasing mic transcription).
    // shared_prompt is still updated for prompt-decay tracking.
    //
    // Truncate to prompt_context_chars for the Whisper prompt (default 350).
    // accumulated_text itself is kept at up to MAX_ACCUMULATED_TEXT_CHARS for the LiveSegmentEvent.
    let max_prompt = ctx.config.prompt_context_chars.unwrap_or(350) as usize;
    let effective_prompt: Option<&str> = if accumulated_text.is_empty() {
        None
    } else if accumulated_text.len() > max_prompt {
        let boundary = accumulated_text.ceil_char_boundary(accumulated_text.len() - max_prompt);
        Some(&accumulated_text[boundary..])
    } else {
        Some(accumulated_text.as_str())
    };

    let transcription_result = {
        let mut client_guard = ctx.whisper_client.lock().await;
        match client_guard.as_mut() {
            Some(client) => {
                client
                    .transcribe(&wav_path, ctx.config.language.as_deref(), effective_prompt)
                    .await
            }
            None => {
                error!("live transcription: whisper client not initialized");
                let _ = ctx.app_handle.emit(
                    "live-transcription-status",
                    LiveTranscriptionStatus {
                        phase: LiveTranscriptionPhase::Error,
                        chunks_processed: *chunk_index,
                        total_audio_seconds: 0.0,
                        error_message: Some("whisper client not initialized".to_string()),
                        session_id: ctx.config.session_id.clone(),
                        effective_start_epoch_ms: None,
                    },
                );
                return TranscribeOutcome::SidecarDead;
            }
        }
    };

    // _wav_guard handles cleanup via Drop

    match transcription_result {
        Ok(result) => {
            *chunk_index += 1;

            // Build prompt-safe text by excluding hallucination patterns.
            // Even if the sidecar accepted segments (e.g. marginal fillers at high
            // confidence), we don't want them priming the next chunk's initial_prompt
            // and creating a feedback loop.
            let prompt_text: String = result
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .filter(|t| !yapstack_common::hallucination::is_always_reject(t))
                .collect::<Vec<_>>()
                .join(" ");

            if !prompt_text.is_empty() {
                if !accumulated_text.is_empty() {
                    accumulated_text.push(' ');
                }
                accumulated_text.push_str(&prompt_text);
                // Cap accumulated_text to prevent unbounded growth over long sessions
                if accumulated_text.len() > MAX_ACCUMULATED_TEXT_CHARS {
                    let boundary = accumulated_text
                        .ceil_char_boundary(accumulated_text.len() - MAX_ACCUMULATED_TEXT_CHARS);
                    *accumulated_text = accumulated_text[boundary..].to_string();
                }
                if !session.shared_prompt.is_empty() {
                    session.shared_prompt.push(' ');
                }
                session.shared_prompt.push_str(&prompt_text);
                let max_prompt = ctx.config.prompt_context_chars.unwrap_or(350) as usize;
                if session.shared_prompt.len() > max_prompt {
                    let boundary = session
                        .shared_prompt
                        .ceil_char_boundary(session.shared_prompt.len() - max_prompt);
                    session.shared_prompt = session.shared_prompt[boundary..].to_string();
                }
            }

            let segments: Vec<TranscriptSegmentDto> = result
                .segments
                .into_iter()
                .map(|s| TranscriptSegmentDto {
                    start_ms: s.start_ms,
                    end_ms: s.end_ms,
                    text: s.text,
                    confidence: s.confidence,
                })
                .collect();

            info!(
                "transcribed: {:?} chunk {} offset={:.2}s {:.1}s audio {} chars",
                input.source_label,
                *chunk_index,
                input.audio_offset_seconds,
                chunk_duration,
                result.text.len()
            );

            TranscribeOutcome::Success(ChunkResult {
                event: LiveSegmentEvent {
                    chunk_index: *chunk_index - 1,
                    source: input.source_label,
                    segments,
                    audio_offset_seconds: input.audio_offset_seconds,
                    chunk_duration_seconds: chunk_duration,
                    accumulated_text: accumulated_text.clone(),
                    is_backfill: input.is_backfill,
                },
                chunk_duration,
            })
        }
        Err(e) => {
            warn!("live transcription: chunk failed: {}, skipping", e);

            // Check if the sidecar process died — attempt auto-restart.
            let mut client_guard = ctx.whisper_client.lock().await;
            if let Some(ref mut client) = *client_guard {
                if !client.is_running() {
                    warn!("sidecar process died — attempting restart");
                    match client.respawn().await {
                        Ok(()) => {
                            info!("sidecar restarted successfully after transcription failure");
                            let _ = ctx.app_handle.emit(
                                "live-transcription-warning",
                                LiveTranscriptionWarningEvent {
                                    message: "Transcription engine restarted".into(),
                                },
                            );
                            return TranscribeOutcome::Skipped;
                        }
                        Err(restart_err) => {
                            error!("sidecar restart failed: {}", restart_err);
                            return TranscribeOutcome::SidecarDead;
                        }
                    }
                }
            }

            TranscribeOutcome::Skipped
        }
    }
}

/// Transcribe a chunk, emit segment/status events, and return the outcome.
///
/// The caller uses the outcome to manage VAD cursor state:
/// - `Success`: advance cursor past transcribed audio
/// - `Skipped`: keep speech_start_pos for retry (audio not lost)
/// - `SidecarDead`: fatal — caller should break the loop
async fn transcribe_and_emit_chunk(
    ctx: &TranscriptionContext,
    input: &ChunkInput<'_>,
    chunk_index: &mut u32,
    accumulated_text: &mut String,
    session: &mut SessionAccumulators,
) -> TranscribeOutcome {
    let outcome = transcribe_chunk(ctx, input, chunk_index, accumulated_text, session).await;

    if let TranscribeOutcome::Success(ref result) = outcome {
        session.total_chunks += 1;
        session.total_audio_seconds += result.chunk_duration;
        session.last_transcription_at = Some(Instant::now());

        let _ = ctx
            .app_handle
            .emit("live-transcription-segment", &result.event);
        emit_status(
            &ctx.app_handle,
            LiveTranscriptionPhase::Running,
            session.total_chunks,
            session.total_audio_seconds,
        );
    }

    outcome
}

// --- Tauri commands ---

#[tauri::command]
#[specta::specta]
pub async fn start_live_transcription(
    audio_state: tauri::State<'_, AudioManagerState>,
    whisper_state: tauri::State<'_, WhisperClientState>,
    live_state: tauri::State<'_, LiveTranscriptionState>,
    app_handle: AppHandle,
    mut config: LiveTranscriptionConfig,
) -> Result<LiveTranscriptionStartResult, CommandError> {
    let mut guard = live_state.lock().await;

    // Check if already running
    if let Some(ref controller) = *guard {
        if controller.is_running() {
            return Err(CommandError::InvalidInput {
                message: "live transcription is already running".into(),
            });
        }
    }

    // Validate config values
    if config.silence_threshold <= 0.0 {
        return Err(CommandError::InvalidInput {
            message: "silence_threshold must be > 0".into(),
        });
    }
    if config.silence_duration_ms == 0 {
        return Err(CommandError::InvalidInput {
            message: "silence_duration_ms must be > 0".into(),
        });
    }
    if config.max_chunk_seconds <= 0.0 {
        return Err(CommandError::InvalidInput {
            message: "max_chunk_seconds must be > 0".into(),
        });
    }
    if config.backfill_seconds < 0.0 {
        return Err(CommandError::InvalidInput {
            message: "backfill_seconds must be >= 0".into(),
        });
    }
    if let Some(decay) = config.prompt_decay_silence_seconds {
        if decay < 0.0 {
            return Err(CommandError::InvalidInput {
                message: "prompt_decay_silence_seconds must be >= 0".into(),
            });
        }
    }

    // Pre-flight: restart any silently-stalled capture streams before we
    // read backfill or spawn the loop. Without this, the first dictation
    // after a long idle (device change, Bluetooth drop, OS sleep) transcribes
    // whatever stale audio happens to be in the ring buffer.
    let preflight_source: CaptureSource = config.source.clone().into();
    preflight_stream_health(audio_state.inner(), &preflight_source, &app_handle).await?;

    // Clamp backfill to actual audio available in the buffer.
    let effective_backfill_seconds = {
        let manager = audio_state.lock().await;
        let source = config.source.clone().into();

        // Log sample rate diagnostic for mixed mode
        if matches!(source, CaptureSource::Mixed) {
            if let (Some(mic), Some(sys)) =
                (manager.mic_buffer_info(), manager.system_buffer_info())
            {
                if mic.sample_rate != sys.sample_rate {
                    info!(
                        "Mixed mode: mic={}Hz/{}ch, system={}Hz/{}ch — will resample during extraction",
                        mic.sample_rate, mic.channels, sys.sample_rate, sys.channels
                    );
                }
            }
        }

        if config.backfill_seconds > 0.0 {
            let mic_avail = if matches!(source, CaptureSource::MicOnly | CaptureSource::Mixed) {
                manager.mic_buffer_info().map(|i| i.available_seconds)
            } else {
                None
            };
            let sys_avail = if matches!(source, CaptureSource::SystemOnly | CaptureSource::Mixed) {
                manager.system_buffer_info().map(|i| i.available_seconds)
            } else {
                None
            };
            let available = match (mic_avail, sys_avail) {
                (Some(m), Some(s)) => m.min(s),
                (Some(v), None) | (None, Some(v)) => v,
                (None, None) => 0.0,
            };
            // 5% safety margin: avoid reading data near the ring buffer write head
            config.backfill_seconds.min(available * 0.95)
        } else {
            0.0
        }
    };

    info!(
        "live transcription starting: requested_backfill={:.1}s effective_backfill={:.1}s",
        config.backfill_seconds, effective_backfill_seconds
    );

    let now_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64;
    let effective_start_epoch_ms = now_unix_ms - effective_backfill_seconds as f64 * 1000.0;

    // Set up streaming WAV writer if session_id is provided
    let session_wav_state = if let Some(ref session_id) = config.session_id {
        validate_session_id(session_id)?;
        let audio_dir = if let Some(ref custom_dir) = config.audio_save_location {
            std::path::PathBuf::from(custom_dir)
        } else {
            app_handle
                .path()
                .app_data_dir()
                .map_err(|e: tauri::Error| CommandError::Internal {
                    message: format!("failed to resolve app data dir: {e}"),
                })?
                .join("audio")
        };
        std::fs::create_dir_all(&audio_dir)?;

        // Determine sample rate from active buffers
        let sample_rate = {
            let manager = audio_state.lock().await;
            manager
                .mic_buffer_info()
                .map(|i| i.sample_rate)
                .or_else(|| manager.system_buffer_info().map(|i| i.sample_rate))
                .unwrap_or(48000)
        };

        let wav_path = audio_dir.join(format!("{session_id}.wav"));
        let writer = yapstack_audio::SessionWavWriter::new(wav_path, sample_rate).map_err(|e| {
            CommandError::Internal {
                message: format!("failed to create session WAV: {e}"),
            }
        })?;

        let source = config.source.clone().into();
        // Force normalize=false for the streaming WAV writer. Per-chunk normalization
        // (every ~300ms) causes jarring volume discontinuities — quiet passages get
        // amplified enormously while loud passages barely change. The user's normalize
        // setting is meant for the real-time preview, not the archival WAV.
        let mix_config = config
            .mix_config
            .as_ref()
            .map(|mc| yapstack_audio::MixConfig {
                mic_gain: mc.mic_gain,
                system_gain: mc.system_gain,
                normalize: false,
            });

        // Rewind flush positions by backfill so the WAV includes backfill audio
        let flush_positions = {
            let manager = audio_state.lock().await;
            let positions = manager.buffer_positions();
            if effective_backfill_seconds > 0.0 {
                let mic_rewind = manager
                    .mic_buffer_info()
                    .map(|i| {
                        let raw = (effective_backfill_seconds
                            * i.sample_rate as f32
                            * i.channels as f32) as usize;
                        // Round down to frame boundary for correct deinterleaving
                        raw - (raw % i.channels as usize)
                    })
                    .unwrap_or(0);
                let sys_rewind = manager
                    .system_buffer_info()
                    .map(|i| {
                        let raw = (effective_backfill_seconds
                            * i.sample_rate as f32
                            * i.channels as f32) as usize;
                        // Round down to frame boundary for correct deinterleaving
                        raw - (raw % i.channels as usize)
                    })
                    .unwrap_or(0);
                BufferPositions {
                    mic_pos: positions.mic_pos.saturating_sub(mic_rewind),
                    system_pos: positions.system_pos.saturating_sub(sys_rewind),
                }
            } else {
                positions
            }
        };

        info!(
            "session WAV writer created: session_id={} sample_rate={}",
            session_id, sample_rate
        );

        Some(SessionWavState {
            writer,
            flush_positions,
            source,
            mix_config,
            session_id: session_id.clone(),
            flush_count: 0,
        })
    } else {
        None
    };

    let (stop_tx, stop_rx) = oneshot::channel();

    let audio_state_clone = audio_state.inner().clone();
    let whisper_state_clone = whisper_state.inner().clone();

    // Align config backfill with the clamped value so WAV writer and transcript
    // cursor share the same time origin (prevents timestamp drift on playback).
    config.backfill_seconds = effective_backfill_seconds;

    // Capture session_id before config is moved into TranscriptionContext
    let controller_session_id = config.session_id.clone();

    // Extract the whisper client only after all fallible setup above succeeds.
    // This avoids losing the client on early-return setup errors.
    let extracted_client = {
        let mut client_guard = whisper_state.lock().await;
        client_guard.take().ok_or(CommandError::NotInitialized {
            message: "whisper client not initialized".into(),
        })?
    };

    let ctx = TranscriptionContext {
        whisper_client: Arc::new(Mutex::new(Some(extracted_client))),
        shared_whisper_state: whisper_state_clone,
        app_handle,
        config,
        bridged_prompt: Arc::new(Mutex::new(String::new())),
    };

    let task_handle = tokio::spawn({
        let ctx_guard = ctx.clone();
        async move {
            let result = AssertUnwindSafe(live_transcription_loop(
                audio_state_clone,
                ctx,
                stop_rx,
                session_wav_state,
            ))
            .catch_unwind()
            .await;

            // Always return the WhisperClient to shared state, even after a panic.
            {
                let mut private_guard = ctx_guard.whisper_client.lock().await;
                if let Some(client) = private_guard.take() {
                    let mut shared_guard = ctx_guard.shared_whisper_state.lock().await;
                    *shared_guard = Some(client);
                    debug!("returned whisper client to shared state");
                }
            }

            if let Err(panic) = result {
                error!("live transcription panicked: {:?}", panic);
                let _ = ctx_guard.app_handle.emit(
                    "live-transcription-status",
                    LiveTranscriptionStatus {
                        phase: LiveTranscriptionPhase::Error,
                        chunks_processed: 0,
                        total_audio_seconds: 0.0,
                        error_message: Some("live transcription crashed unexpectedly".to_string()),
                        session_id: ctx_guard.config.session_id.clone(),
                        effective_start_epoch_ms: None,
                    },
                );
            }
        }
    });

    *guard = Some(LiveTranscriptionController {
        task_handle,
        stop_tx: Some(stop_tx),
        session_id: controller_session_id,
        effective_start_epoch_ms,
    });

    Ok(LiveTranscriptionStartResult {
        effective_start_epoch_ms,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn stop_live_transcription(
    live_state: tauri::State<'_, LiveTranscriptionState>,
) -> Result<(), CommandError> {
    let mut guard = live_state.lock().await;

    match guard.take() {
        Some(mut controller) => {
            if let Some(tx) = controller.stop_tx.take() {
                let _ = tx.send(());
            }
            // Don't await the task — it will finish on its own and emit Stopped status
            Ok(())
        }
        None => Err(CommandError::InvalidInput {
            message: "live transcription is not running".into(),
        }),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_live_transcription_status(
    live_state: tauri::State<'_, LiveTranscriptionState>,
) -> Result<LiveTranscriptionStatus, CommandError> {
    let guard = live_state.lock().await;

    // TODO: chunks_processed and total_audio_seconds are always 0 because these
    // counters live inside the async transcription loop (local variables in
    // `live_transcription_loop`). To report real values, the loop would need to
    // write to an Arc<AtomicU32> / Arc<AtomicF32> (or a shared struct) that this
    // command reads. Low priority since the frontend receives accurate per-chunk
    // values via the "live-transcription-status" event stream.
    match &*guard {
        Some(controller) => {
            if controller.is_running() {
                Ok(LiveTranscriptionStatus {
                    phase: LiveTranscriptionPhase::Running,
                    chunks_processed: 0,
                    total_audio_seconds: 0.0,
                    error_message: None,
                    session_id: controller.session_id.clone(),
                    effective_start_epoch_ms: Some(controller.effective_start_epoch_ms),
                })
            } else {
                Ok(LiveTranscriptionStatus {
                    phase: LiveTranscriptionPhase::Stopped,
                    chunks_processed: 0,
                    total_audio_seconds: 0.0,
                    error_message: None,
                    session_id: controller.session_id.clone(),
                    effective_start_epoch_ms: Some(controller.effective_start_epoch_ms),
                })
            }
        }
        None => Ok(LiveTranscriptionStatus {
            phase: LiveTranscriptionPhase::Stopped,
            chunks_processed: 0,
            total_audio_seconds: 0.0,
            error_message: None,
            session_id: None,
            effective_start_epoch_ms: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_leading_silence_all_loud() {
        // Signal well above threshold
        let samples: Vec<f32> = vec![0.5; 1600]; // 0.1s at 16kHz
        let (trimmed, offset) = trim_leading_silence(&samples, 16000, 0.01, 0.0);
        assert_eq!(trimmed.len(), 1600);
        assert!((offset - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_trim_leading_silence_all_silent() {
        // All zeros — below any threshold
        let samples: Vec<f32> = vec![0.0; 1600];
        let (trimmed, offset) = trim_leading_silence(&samples, 16000, 0.01, 0.0);
        // Returns empty slice when all silent so caller can skip
        assert!(trimmed.is_empty());
        assert!((offset - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_trim_leading_silence_with_leading_silence() {
        // 0.1s silence then 0.1s loud at 16kHz
        let mut samples = vec![0.0f32; 1600];
        samples.extend(vec![0.5f32; 1600]);
        let (trimmed, offset) = trim_leading_silence(&samples, 16000, 0.01, 0.0);
        // Should trim the leading silence
        assert!(trimmed.len() < 3200);
        assert!(offset > 0.0);
    }

    #[test]
    fn test_trim_leading_silence_with_pad() {
        // 0.2s silence then loud at 16kHz
        let mut samples = vec![0.0f32; 3200];
        samples.extend(vec![0.5f32; 1600]);
        // Pad 0.05s before the loud section
        let (trimmed, offset) = trim_leading_silence(&samples, 16000, 0.01, 0.05);
        // Offset should be slightly less than the loud section start (due to padding)
        assert!(offset > 0.0);
        assert!(trimmed.len() > 1600); // includes some pad
    }

    #[test]
    fn test_trim_leading_silence_empty() {
        let samples: Vec<f32> = vec![];
        let (trimmed, offset) = trim_leading_silence(&samples, 16000, 0.01, 0.0);
        assert!(trimmed.is_empty());
        assert!((offset - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_capture_source_dto_into() {
        let source: CaptureSource = CaptureSourceDto::MicOnly.into();
        assert_eq!(source, CaptureSource::MicOnly);
        let source: CaptureSource = CaptureSourceDto::SystemOnly.into();
        assert_eq!(source, CaptureSource::SystemOnly);
        let source: CaptureSource = CaptureSourceDto::Mixed.into();
        assert_eq!(source, CaptureSource::Mixed);
    }

    // --- VAD state machine tests ---

    /// Shorthand for poll_vad with standard test thresholds.
    fn poll(state: &mut SourceVadState, energy: Option<f32>) -> VadAction {
        poll_vad(
            state,
            energy,
            0.01,
            Duration::from_millis(800),
            Duration::from_secs(30),
        )
    }

    #[test]
    fn test_poll_vad_silence_returns_none() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        // Below threshold while not speaking → None, stays not-speaking
        let action = poll(&mut state, Some(0.005));
        assert!(matches!(action, VadAction::None));
        assert!(!state.is_speaking);
    }

    #[test]
    fn test_poll_vad_speech_onset() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        // Above threshold while not speaking → transitions to speaking
        let action = poll(&mut state, Some(0.05));
        assert!(matches!(action, VadAction::None));
        assert!(state.is_speaking);
        assert!(state.speech_start_time.is_some());
    }

    #[test]
    fn test_poll_vad_loud_clears_silence_timer() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        state.is_speaking = true;
        state.speech_start_time = Some(Instant::now());
        state.silence_since = Some(Instant::now()); // had some silence accumulating
                                                    // Above threshold while speaking, below max duration → clears silence_since
        let action = poll(&mut state, Some(0.05));
        assert!(matches!(action, VadAction::None));
        assert!(state.silence_since.is_none());
    }

    #[test]
    fn test_poll_vad_energy_at_exact_threshold() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        // Exactly at threshold (0.01) while not speaking — uses >=, so SHOULD trigger onset
        let action = poll(&mut state, Some(0.01));
        assert!(matches!(action, VadAction::None));
        assert!(state.is_speaking);
        assert!(state.speech_start_time.is_some());
    }

    #[test]
    fn test_poll_vad_silence_begins() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        state.is_speaking = true;
        state.speech_start_time = Some(Instant::now());
        // Below threshold while speaking → sets silence_since but not long enough yet
        let action = poll(&mut state, Some(0.005));
        assert!(matches!(action, VadAction::None));
        assert!(state.silence_since.is_some());
    }

    #[test]
    fn test_poll_vad_silence_triggers_chunk() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        state.is_speaking = true;
        state.speech_start_time = Some(Instant::now());
        // Silence started longer ago than the threshold
        state.silence_since = Some(Instant::now() - Duration::from_secs(2));
        let action = poll(&mut state, Some(0.005));
        assert!(matches!(action, VadAction::Chunk));
    }

    #[test]
    fn test_poll_vad_force_chunk_max_duration() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        state.is_speaking = true;
        state.speech_start_time = Some(Instant::now() - Duration::from_secs(31));
        // Above threshold while speaking past max_chunk_duration → ForceChunk
        let action = poll(&mut state, Some(0.05));
        assert!(matches!(action, VadAction::ForceChunk));
    }

    #[test]
    fn test_poll_vad_none_energy() {
        // energy = None → always None regardless of state
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        let action = poll(&mut state, None);
        assert!(matches!(action, VadAction::None));

        // Also when speaking
        state.is_speaking = true;
        state.speech_start_time = Some(Instant::now());
        let action = poll(&mut state, None);
        assert!(matches!(action, VadAction::None));
    }

    // --- build_cursor tests ---

    // --- Prompt decay tests ---

    #[test]
    fn test_prompt_decay_clears_after_timeout() {
        let mut sources = vec![SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1)];
        let mut prompt = "some prior transcript context".to_string();
        let last_transcription_at = Some(Instant::now() - Duration::from_secs(6));
        let cleared = check_prompt_decay(&mut sources, &mut prompt, 5.0, last_transcription_at);
        assert!(cleared);
        assert!(prompt.is_empty());
    }

    #[test]
    fn test_prompt_decay_no_clear_with_recent_transcription() {
        let mut sources = vec![SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1)];
        let mut prompt = "some context".to_string();
        // Transcription just happened — should not decay
        let last_transcription_at = Some(Instant::now());
        let cleared = check_prompt_decay(&mut sources, &mut prompt, 5.0, last_transcription_at);
        assert!(!cleared);
        assert_eq!(prompt, "some context");
    }

    #[test]
    fn test_prompt_decay_no_clear_before_timeout() {
        let mut sources = vec![SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1)];
        let mut prompt = "some context".to_string();
        let last_transcription_at = Some(Instant::now() - Duration::from_secs(2));
        let cleared = check_prompt_decay(&mut sources, &mut prompt, 5.0, last_transcription_at);
        assert!(!cleared);
        assert_eq!(prompt, "some context");
    }

    #[test]
    fn test_prompt_decay_disabled_when_zero() {
        let mut sources = vec![SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1)];
        let mut prompt = "some context".to_string();
        let last_transcription_at = Some(Instant::now() - Duration::from_secs(60));
        let cleared = check_prompt_decay(&mut sources, &mut prompt, 0.0, last_transcription_at);
        assert!(!cleared);
        assert_eq!(prompt, "some context");
    }

    #[test]
    fn test_prompt_decay_already_empty() {
        let mut sources = vec![SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1)];
        let mut prompt = String::new();
        let last_transcription_at = Some(Instant::now() - Duration::from_secs(10));
        let cleared = check_prompt_decay(&mut sources, &mut prompt, 5.0, last_transcription_at);
        assert!(!cleared);
    }

    #[test]
    fn test_prompt_decay_clears_accumulated_text() {
        let mut sources = vec![
            SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1),
            SourceVadState::new(AudioSourceLabel::System, 0, 0, 48000, 2),
        ];
        sources[0].accumulated_text = "mic transcript context".to_string();
        sources[1].accumulated_text = "system transcript context".to_string();
        let mut prompt = "shared prompt".to_string();
        let last_transcription_at = Some(Instant::now() - Duration::from_secs(6));
        let cleared = check_prompt_decay(&mut sources, &mut prompt, 5.0, last_transcription_at);
        assert!(cleared);
        assert!(prompt.is_empty());
        assert!(sources[0].accumulated_text.is_empty());
        assert!(sources[1].accumulated_text.is_empty());
    }

    #[test]
    fn test_prompt_decay_triggers_on_accumulated_text_only() {
        // shared_prompt is empty but accumulated_text is not — should still trigger
        let mut sources = vec![SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1)];
        sources[0].accumulated_text = "stale context from before silence".to_string();
        let mut prompt = String::new();
        let last_transcription_at = Some(Instant::now() - Duration::from_secs(6));
        let cleared = check_prompt_decay(&mut sources, &mut prompt, 5.0, last_transcription_at);
        assert!(cleared);
        assert!(sources[0].accumulated_text.is_empty());
    }

    #[test]
    fn test_prompt_decay_no_clear_when_never_transcribed() {
        // No transcription has ever occurred — prompt may have been seeded
        // but last_transcription_at is None → no decay
        let mut sources = vec![SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1)];
        sources[0].accumulated_text = "seeded context".to_string();
        let mut prompt = "shared prompt".to_string();
        let cleared = check_prompt_decay(&mut sources, &mut prompt, 5.0, None);
        assert!(!cleared);
        assert_eq!(prompt, "shared prompt");
        assert_eq!(sources[0].accumulated_text, "seeded context");
    }

    // --- build_cursor tests ---

    #[test]
    fn test_build_cursor_mic_only() {
        let sources = vec![SourceVadState::new(AudioSourceLabel::Mic, 42, 0, 48000, 1)];
        let cursor = build_cursor(&sources);
        assert_eq!(cursor.mic_pos, 42);
        assert_eq!(cursor.system_pos, 0);
    }

    #[test]
    fn test_build_cursor_both_sources() {
        let sources = vec![
            SourceVadState::new(AudioSourceLabel::Mic, 100, 0, 48000, 1),
            SourceVadState::new(AudioSourceLabel::System, 200, 0, 48000, 2),
        ];
        let cursor = build_cursor(&sources);
        assert_eq!(cursor.mic_pos, 100);
        assert_eq!(cursor.system_pos, 200);
    }

    // --- chunk_at_silence_boundaries tests ---

    #[test]
    fn test_chunk_at_silence_empty() {
        let chunks = chunk_at_silence_boundaries(&[], 48000, 48000, 0.01);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_at_silence_short_audio() {
        // Audio shorter than chunk_size → single chunk
        let samples = vec![0.5f32; 16000]; // 1s at 16kHz
        let chunks = chunk_at_silence_boundaries(&samples, 48000, 16000, 0.01);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 16000);
    }

    #[test]
    fn test_chunk_at_silence_splits_at_silence() {
        // Build audio: 2s loud, 0.2s silence, 2s loud at 16kHz
        let sr = 16000;
        let loud_len = 2 * sr;
        let silence_len = (0.2 * sr as f32) as usize;
        let mut samples = vec![0.5f32; loud_len];
        samples.extend(vec![0.0f32; silence_len]);
        samples.extend(vec![0.5f32; loud_len]);
        // Total: ~4.2s. Chunk size: 3s (48000 samples at 16kHz).
        let chunk_size = 3 * sr;
        let chunks = chunk_at_silence_boundaries(&samples, chunk_size, sr as u32, 0.01);
        // Should split near the silence gap rather than at the hard 3s boundary
        assert!(
            chunks.len() >= 2,
            "expected at least 2 chunks, got {}",
            chunks.len()
        );
        // The total samples across all chunks should equal the input
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, samples.len());
    }

    #[test]
    fn test_chunk_at_silence_no_silence_falls_back() {
        // Continuous loud audio — should fall back to fixed boundaries
        let sr = 16000usize;
        let samples = vec![0.5f32; 5 * sr]; // 5s loud
        let chunk_size = 2 * sr; // 2s chunks
        let chunks = chunk_at_silence_boundaries(&samples, chunk_size, sr as u32, 0.01);
        assert!(chunks.len() >= 2);
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, samples.len());
    }

    // --- Stream health decision helper tests ---

    #[test]
    fn test_should_stall_restart_mic_always_true() {
        // Mic stall should always trigger restart on all platforms
        assert!(should_stall_restart(&AudioSourceLabel::Mic));
    }

    #[test]
    fn test_should_stall_restart_system_platform_dependent() {
        let result = should_stall_restart(&AudioSourceLabel::System);
        if cfg!(target_os = "windows") {
            // On Windows, system stall should NOT trigger restart (WASAPI silence)
            assert!(!result, "system stall restart should be skipped on Windows");
        } else {
            // On other platforms, system stall should trigger restart
            assert!(
                result,
                "system stall restart should be enabled on non-Windows"
            );
        }
    }

    #[test]
    fn test_classify_empty_flush_table() {
        // Table-driven: (source, has_stream_error, expected)
        let cases = [
            // MicOnly is always an error regardless of stream error flag
            (
                CaptureSource::MicOnly,
                false,
                EmptyFlushClassification::Error,
            ),
            (
                CaptureSource::MicOnly,
                true,
                EmptyFlushClassification::Error,
            ),
            // Mixed is always an error (both sources should produce data)
            (CaptureSource::Mixed, false, EmptyFlushClassification::Error),
            (CaptureSource::Mixed, true, EmptyFlushClassification::Error),
            // SystemOnly without stream error: WindowsSilence on Windows, Error elsewhere
            (
                CaptureSource::SystemOnly,
                false,
                if cfg!(target_os = "windows") {
                    EmptyFlushClassification::WindowsSilence
                } else {
                    EmptyFlushClassification::Error
                },
            ),
            // SystemOnly WITH stream error: always Error (real failure)
            (
                CaptureSource::SystemOnly,
                true,
                EmptyFlushClassification::Error,
            ),
        ];

        for (i, (source, has_error, expected)) in cases.iter().enumerate() {
            let result = classify_empty_flush(*source, *has_error);
            assert_eq!(
                result, *expected,
                "case {}: source={:?} has_error={} expected {:?} got {:?}",
                i, source, has_error, expected, result
            );
        }
    }
}
