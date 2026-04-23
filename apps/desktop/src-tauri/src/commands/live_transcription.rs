use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
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
use super::silero_vad::{SileroSource, SileroVad, SILENCE_THRESHOLD, SPEECH_THRESHOLD};
use super::transcription::{TranscriptSegmentDto, TranscriptionClientState};

// --- DTOs ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
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
    /// Audio export format: "wav" or "mp3". Default: "mp3".
    pub audio_export_format: Option<String>,
    /// MP3 bitrate in kbps (e.g. 64, 128, 192). Only used when format is "mp3".
    pub mp3_bitrate: Option<u16>,
    /// Request speaker diarization on every transcribed chunk. Honored only
    /// when the active engine is Parakeet *and* the sidecar was spawned with
    /// a Sortformer model path. Whisper sessions ignore this flag.
    #[serde(default)]
    pub diarization: bool,
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
    /// Session this chunk belongs to. Carried on the event so late-arriving
    /// segments can still be persisted to the right session even if the
    /// frontend has already cleared `activeSessionId` during finalization.
    pub session_id: Option<String>,
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
/// During live transcription, the TranscriptionClient is extracted from the
/// shared `TranscriptionClientState` and held privately in
/// `transcription_client`. This eliminates mutex contention with other
/// commands that may access `TranscriptionClientState` (e.g.
/// `transcribe_audio`, `shutdown_transcription_client`). The client is
/// returned to shared state when the live transcription loop ends.
#[derive(Clone)]
struct TranscriptionContext {
    /// Private client for the live transcription loop. Wrapped in an inner
    /// `Arc` so concurrent chunk tasks (mic + system in the same poll tick)
    /// can each briefly lock the outer mutex, clone the Arc, and then hold
    /// the client by reference across their transcribe await without
    /// contending on the outer mutex. The inner `TranscriptionClient` is
    /// concurrent-safe by construction (per-id oneshot response routing).
    ///
    /// The `Option` layer remains so we can move the raw client back into
    /// shared state on session end via `Arc::try_unwrap`.
    transcription_client: Arc<Mutex<Option<Arc<yapstack_transcription::TranscriptionClient>>>>,
    /// Reference to the shared state so we can return the client when done.
    shared_transcription_state: TranscriptionClientState,
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

/// Outcome of a transcription attempt — determines cursor management and
/// loop control. Too-short and empty extractions are filtered before dispatch
/// in `prepare_chunk_dispatch`, so we don't need a TooShort variant here.
enum TranscribeOutcome {
    /// Transcription succeeded with segments.
    Success(ChunkResult),
    /// Transcription failed non-fatally (temp WAV write error, engine error,
    /// timeout). The fire-and-forget dispatch path quarantines the audio
    /// immediately — retries are not attempted in the concurrent design.
    Skipped,
    /// Sidecar is dead and could not be restarted — main loop exits.
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
///
/// Shared across concurrent per-source chunk tasks via `Arc<StdMutex<_>>`.
/// All mutation sites lock briefly; the mutex is never held across an await.
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
#[allow(dead_code)] // retained as the pair with chunk_at_silence_boundaries
struct BackfillChunk<'a> {
    samples: &'a [f32],
    sample_rate: u32,
}

/// One source's backfill data: the raw samples plus VAD-simulated chunk
/// boundaries (start/end sample indices) produced by `vad_chunk_historical_audio`.
struct VadBackfillSource {
    label: AudioSourceLabel,
    samples: Vec<f32>,
    sample_rate: u32,
    chunks: Vec<VadBackfillChunk>,
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
    /// Last observed write_pos for stall detection.
    last_seen_write_pos: usize,
    /// When last_seen_write_pos last advanced.
    last_write_pos_advance: Instant,
    /// Number of restart attempts for this source.
    restart_attempts: u32,
    /// When the last restart was attempted (for cooldown).
    last_restart_at: Option<Instant>,
    /// True while a background chunk task is running for this source. VAD
    /// state keeps updating (so second-utterance-during-in-flight is still
    /// captured) but dispatch and idle-cursor advance are gated off.
    /// Cleared when the task completes and the main loop applies its outcome.
    has_in_flight_task: bool,
    /// Lower bound for onset pre-roll adjustments. Prevents pre-roll from
    /// pulling `speech_start_pos` before the last dispatched chunk's end,
    /// which would cause the next chunk to re-transcribe the tail of the
    /// previous one. Set to `new_pos` at every chunk dispatch and to
    /// `session_start_pos` at loop entry (no prior dispatch to respect).
    earliest_next_chunk_pos: usize,
    /// Silero VAD streaming state and VAD-only read cursor for this source.
    /// Populated regardless of engine — Silero replaces RMS for both
    /// Whisper and Parakeet live sessions.
    silero: SileroSource,
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
            has_in_flight_task: false,
            earliest_next_chunk_pos: session_start_pos,
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
            last_seen_write_pos: initial_pos,
            last_write_pos_advance: Instant::now(),
            restart_attempts: 0,
            last_restart_at: None,
            silero: SileroSource::new(initial_pos),
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

/// Per-engine tuning for the VAD / chunking loop.
///
/// The detector itself (Silero VAD V5) is shared across engines — it's a
/// speech-trained neural model that outperforms the old RMS threshold on
/// music, keyboard clicks, HVAC noise, and quiet speech. Only the
/// *timing* knobs are engine-specific:
///
/// - Whisper: the original dictation-tuned cadence (honors the user's
///   frontend `silence_duration_ms`, 300 ms poll, no pre-roll). Values
///   preserve the proven Whisper dictation feel — Silero only changes
///   *what* we detect as speech, not *how long* we wait before chunking.
/// - Parakeet: dialogue-aggressive cadence (200 ms silence, 10 s max
///   chunk, 100 ms poll, 250 ms pre-roll). Parakeet's low RTFx and
///   non-autoregressive decoder make short chunks free; we bias for
///   responsive on-screen updates.
///
/// `speech_threshold` / `offset_threshold_ratio` are now Silero
/// *probability* thresholds (V5 defaults: 0.5 / 0.35) rather than RMS
/// energy thresholds. They're held per-tuning so a future engine could
/// override them if needed, but today both engines use the same values.
#[derive(Debug, Clone, Copy)]
struct VadTuning {
    /// Speech-probability threshold at or above which a frame is
    /// considered speech (Silero V5 default 0.5).
    speech_threshold: f32,
    /// End-of-speech threshold as a ratio of `speech_threshold`. Must be
    /// in (0.0, 1.0]. 0.7 yields the V5-default 0.35 end threshold
    /// (hysteresis — prevents flapping on short probability dips).
    offset_threshold_ratio: f32,
    /// How long silence must hold before chunking.
    silence_duration: Duration,
    /// Force-chunk after this much continuous speech.
    max_chunk_duration: Duration,
    /// Inner poll cadence for VAD probability checks + transitions.
    poll_interval: Duration,
    /// Pre-onset capture: extraction starts this far *before* the point we
    /// noticed energy cross the threshold, to catch leading plosives.
    pre_roll: Duration,
}

fn vad_tuning_for(
    engine: yapstack_common::types::EngineKind,
    config: &LiveTranscriptionConfig,
) -> VadTuning {
    use yapstack_common::types::EngineKind;
    match engine {
        // Whisper: preserve existing dictation-proven *timing* exactly.
        // Silence window honors the user's `silence_duration_ms` (frontend
        // default 800 ms); 300 ms poll cadence; no pre-roll. Only the
        // detector swaps RMS → Silero — all timing constants stay put.
        EngineKind::Whisper => VadTuning {
            speech_threshold: SPEECH_THRESHOLD,
            offset_threshold_ratio: SILENCE_THRESHOLD / SPEECH_THRESHOLD,
            silence_duration: Duration::from_millis(config.silence_duration_ms as u64),
            max_chunk_duration: Duration::from_secs_f32(config.max_chunk_seconds),
            poll_interval: Duration::from_millis(POLL_INTERVAL_MS),
            pre_roll: Duration::ZERO,
        },
        // Parakeet: dialogue-aggressive. Ignores frontend silence / chunk /
        // poll knobs — those are engine-specific best practice, not
        // user-facing tuning. See live_transcription docs for rationale.
        EngineKind::Parakeet => VadTuning {
            speech_threshold: SPEECH_THRESHOLD,
            offset_threshold_ratio: SILENCE_THRESHOLD / SPEECH_THRESHOLD,
            silence_duration: Duration::from_millis(200),
            max_chunk_duration: Duration::from_secs(10),
            poll_interval: Duration::from_millis(100),
            pre_roll: Duration::from_millis(250),
        },
    }
}

/// Polls the VAD state machine for a single source. The `probability`
/// input is Silero's per-frame speech probability in [0, 1]; `None`
/// means no full frame accumulated during this poll window (in which
/// case the state machine stays put — no toggles on missing data).
/// Manages `speech_start_time` internally on state transitions.
/// Uses `tuning.speech_threshold` for onset detection and
/// `tuning.speech_threshold * tuning.offset_threshold_ratio` for offset
/// (hysteresis prevents mid-word dropout on short probability dips).
fn poll_vad(state: &mut SourceVadState, probability: Option<f32>, tuning: &VadTuning) -> VadAction {
    let Some(probability) = probability else {
        return VadAction::None;
    };

    if state.is_speaking {
        let offset_threshold = tuning.speech_threshold * tuning.offset_threshold_ratio;
        let is_loud = probability >= offset_threshold;
        if is_loud {
            state.silence_since = None;

            let speech_elapsed = state
                .speech_start_time
                .map(|t| t.elapsed())
                .unwrap_or_default();
            if speech_elapsed >= tuning.max_chunk_duration {
                return VadAction::ForceChunk;
            }

            VadAction::None
        } else {
            let silence_start = *state.silence_since.get_or_insert_with(Instant::now);
            if silence_start.elapsed() >= tuning.silence_duration {
                VadAction::Chunk
            } else {
                VadAction::None
            }
        }
    } else {
        let is_loud = probability >= tuning.speech_threshold;
        if is_loud {
            state.is_speaking = true;
            state.silence_since = None;
            state.speech_start_time = Some(Instant::now());
            // Pre-roll: rewind speech_start_pos so the extracted chunk
            // includes the audio immediately *before* we noticed energy
            // cross the threshold. Clamped to `earliest_next_chunk_pos`
            // (last dispatched chunk's end, or session_start_pos when no
            // chunk has been dispatched yet) so we never re-transcribe the
            // tail of a previous chunk when onset happens soon after a
            // dispatch — e.g. a second utterance while the previous
            // chunk's task is still in flight.
            if tuning.pre_roll > Duration::ZERO {
                let pre_roll_samples = (tuning.pre_roll.as_secs_f32()
                    * state.source_sample_rate as f32
                    * state.source_channels as f32) as usize;
                state.speech_start_pos = state
                    .speech_start_pos
                    .saturating_sub(pre_roll_samples)
                    .max(state.earliest_next_chunk_pos);
            }
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

/// Reconstruct `BufferPositions` from the source vec. Retained as a
/// general helper and exercised by tests; the live loop no longer needs
/// it now that Silero VAD extracts samples per-source via `extract_source_audio`.
#[allow(dead_code)]
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

/// Outcome returned from a background chunk task back to the main loop so it
/// can restore per-source state (accumulated_text) and notice fatal failures.
struct ChunkTaskOutcome {
    source_label: AudioSourceLabel,
    /// Returned so the main loop can write it back to `vad.accumulated_text`.
    /// The task mutates a moved-in copy; moving back on completion is safe
    /// because we gate VAD polling on `has_in_flight_task` — no concurrent
    /// reader of `vad.accumulated_text` exists while the task runs.
    accumulated_text: String,
    /// True when the sidecar died and could not be restarted. Main loop
    /// exits on receiving this.
    sidecar_dead: bool,
}

/// Prepare a chunk for background transcription: extract audio, advance the
/// source's VAD cursor/state optimistically (as if the task will succeed),
/// and return the parameters needed to spawn the task. Returns `None` when
/// there's no audio to transcribe yet (too short / empty extraction) so the
/// caller skips task spawn and the next poll re-tries.
///
/// Sets `vad.has_in_flight_task = true` when a task is dispatched. The main
/// loop clears it when the task's outcome lands.
async fn prepare_chunk_dispatch(
    vad: &mut SourceVadState,
    audio_state: &AudioManagerState,
    is_force_chunk: bool,
    is_backfill: bool,
) -> Option<PreparedChunk> {
    let (extraction, new_pos) = {
        let manager = audio_state.lock().await;
        extract_source_audio(&manager, &vad.label, vad.speech_start_pos)
    };

    let Some((samples, sample_rate)) = extraction else {
        // Nothing new in the buffer since last read — advance the cursor to
        // the latest write_pos but leave speech_start_pos so we'll pick up
        // the ongoing utterance on the next poll.
        vad.cursor = new_pos;
        return None;
    };

    let chunk_duration = samples.len() as f32 / sample_rate as f32;
    if chunk_duration < MIN_CHUNK_DURATION_SECS {
        // Too short — don't dispatch yet and don't advance speech_start_pos.
        // Next poll re-extracts this region together with whatever arrives.
        vad.cursor = new_pos;
        return None;
    }

    // Deterministic offset from buffer position delta.
    let samples_since_start = vad.speech_start_pos.saturating_sub(vad.session_start_pos);
    let audio_offset =
        samples_since_start as f32 / (vad.source_sample_rate as f32 * vad.source_channels as f32);

    debug!(
        "live chunk: source={:?} offset={:.2}s duration={:.2}s samples={} (pos: speech={} session_start={}, in-flight dispatch)",
        vad.label,
        audio_offset,
        chunk_duration,
        samples.len(),
        vad.speech_start_pos,
        vad.session_start_pos,
    );

    // Snapshot chunk_index for this task, increment our counter so the next
    // chunk (if any) gets a unique id. The task emits events carrying this
    // index; we don't need to write back.
    let chunk_index = vad.chunk_index;
    vad.chunk_index = vad.chunk_index.wrapping_add(1);

    // Move accumulated_text into the task; the main loop restores it on
    // outcome. Safe because VAD polling is gated off this source until then.
    let accumulated_text = std::mem::take(&mut vad.accumulated_text);

    // Optimistically advance cursor + VAD state as if the task will succeed.
    // On transient failures we quarantine the audio rather than retry the
    // same region — the main loop continues unblocked, and the user gets a
    // visible warning with a recoverable WAV path.
    vad.cursor = new_pos;
    vad.is_speaking = false;
    vad.silence_since = None;
    vad.speech_start_pos = new_pos;
    // Pre-roll for any subsequent onset must not pull speech_start_pos
    // before this point, or the next chunk would redundantly transcribe
    // the tail of the audio this task is about to send.
    vad.earliest_next_chunk_pos = new_pos;
    if is_force_chunk {
        vad.speech_start_time = Some(Instant::now());
    } else {
        vad.speech_start_time = None;
    }

    vad.has_in_flight_task = true;

    Some(PreparedChunk {
        samples,
        sample_rate,
        audio_offset_seconds: audio_offset,
        source_label: vad.label,
        chunk_index,
        accumulated_text,
        is_backfill,
    })
}

/// Owned chunk data handed off to a spawned background task. Contains
/// everything the task needs; no references into the main-loop state.
struct PreparedChunk {
    samples: Vec<f32>,
    sample_rate: u32,
    audio_offset_seconds: f32,
    source_label: AudioSourceLabel,
    chunk_index: u32,
    accumulated_text: String,
    is_backfill: bool,
}

/// Run a single chunk's transcription in a background task. Emits the
/// segment event on success; quarantines the audio and emits a warning on
/// failure. Returns the outcome so the main loop can restore
/// `accumulated_text` and notice a dead sidecar.
async fn run_chunk_task(
    prepared: PreparedChunk,
    ctx: TranscriptionContext,
    session: Arc<StdMutex<SessionAccumulators>>,
) -> ChunkTaskOutcome {
    let source_label = prepared.source_label;
    let chunk_index_for_quarantine = prepared.chunk_index;
    let samples_snapshot = prepared.samples.clone();
    let sample_rate_snapshot = prepared.sample_rate;

    let input = ChunkInput {
        samples: &prepared.samples,
        sample_rate: prepared.sample_rate,
        audio_offset_seconds: prepared.audio_offset_seconds,
        source_label,
        is_backfill: prepared.is_backfill,
    };
    let mut chunk_index_mut = prepared.chunk_index;
    let mut accumulated_text = prepared.accumulated_text;

    let outcome = transcribe_and_emit_chunk(
        &ctx,
        &input,
        &mut chunk_index_mut,
        &mut accumulated_text,
        &session,
    )
    .await;

    let sidecar_dead = matches!(outcome, TranscribeOutcome::SidecarDead);
    let skipped = matches!(outcome, TranscribeOutcome::Skipped);

    if skipped {
        // Transient failure (temp WAV write error, engine error, timeout).
        // Quarantine the audio immediately so the user can recover it, and
        // emit a visible warning. No retry — retry-in-place doesn't help
        // when the failure is persistent, and concurrent dispatch means the
        // main loop isn't blocked waiting to retry.
        let duration_s = samples_snapshot.len() as f32 / sample_rate_snapshot as f32;
        let quarantine_path = quarantine_chunk_to_wav(
            &ctx.app_handle,
            ctx.config.session_id.as_deref(),
            &source_label,
            chunk_index_for_quarantine,
            &samples_snapshot,
            sample_rate_snapshot,
        );
        let path_str = quarantine_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<no audio captured>".to_string());
        warn!(
            "live transcription: chunk failed for source {:?} — quarantined {:.2}s to {}",
            source_label, duration_s, path_str
        );
        let _ = ctx.app_handle.emit(
            "live-transcription-warning",
            LiveTranscriptionWarningEvent {
                message: format!(
                    "{:.1}s of audio could not be transcribed; saved for debugging at {}",
                    duration_s, path_str
                ),
            },
        );
    }

    ChunkTaskOutcome {
        source_label,
        accumulated_text,
        sidecar_dead,
    }
}

/// Save an abandoned chunk's audio to the quarantine directory so the user/dev
/// can inspect what was lost when retries exhausted. Returns the file path on
/// success; logs (without the `?` operator) and returns `None` on any failure
/// — quarantine is best-effort and must never break the live loop.
fn quarantine_chunk_to_wav(
    app: &AppHandle,
    session_id: Option<&str>,
    label: &AudioSourceLabel,
    chunk_index: u32,
    samples: &[f32],
    sample_rate: u32,
) -> Option<std::path::PathBuf> {
    let base = match app.path().app_data_dir() {
        Ok(d) => d.join("audio").join("quarantine"),
        Err(e) => {
            warn!("quarantine: cannot resolve app_data_dir: {}", e);
            return None;
        }
    };
    let dir = match session_id {
        Some(sid) => base.join(sid),
        None => base.join("dictation"),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!("quarantine: cannot create dir {}: {}", dir.display(), e);
        return None;
    }
    let source_str = match label {
        AudioSourceLabel::Mic => "mic",
        AudioSourceLabel::System => "system",
    };
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    let path = dir.join(format!("{ts}_{source_str}_{chunk_index}.wav"));
    if let Err(e) = yapstack_audio::export::write_wav(samples, sample_rate, 1, &path) {
        warn!("quarantine: write_wav failed at {}: {}", path.display(), e);
        return None;
    }
    Some(path)
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
#[allow(dead_code)] // kept for tests + potential fallback path
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

    let (mic_initial, sys_initial, mic_err, sys_err, has_mic_buf, has_sys_buf) = {
        let m = audio_state.lock().await;
        (
            m.mic_write_pos(),
            m.system_write_pos(),
            m.mic_has_stream_error(),
            m.system_has_stream_error(),
            m.mic_buffer().is_some(),
            m.system_buffer().is_some(),
        )
    };

    let check_mic = check_mic && has_mic_buf;
    let check_system = check_system && has_sys_buf;

    if !check_mic && !check_system {
        return Ok(());
    }

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

/// One chunk produced by the backfill VAD simulation — carries the start
/// sample index (for computing audio_offset_seconds relative to the backfill
/// window) and the sample slice range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VadBackfillChunk {
    start: usize,
    end: usize,
}

/// Simulate the live VAD state machine over a historical audio buffer and
/// produce chunks with the same boundaries the live loop would have picked.
/// Using the same `VadTuning` the live loop uses gives backfill and live
/// segmentation identical character — a session's first N seconds no longer
/// look subtly different from the rest.
///
/// Runs Silero VAD across the buffer up-front (one probability per 32 ms
/// frame), then walks the probability stream with the same state machine
/// the live loop uses:
/// - is_speaking (onset at `tuning.speech_threshold`, offset at
///   `tuning.speech_threshold * tuning.offset_threshold_ratio` — hysteresis)
/// - silence_run / speech_run counted in Silero frames
/// - pre-roll rewinds the chunk start by `tuning.pre_roll`
///
/// Any trailing speech that didn't naturally end in silence is emitted as a
/// final chunk so the tail of the backfill window is never dropped. If
/// Silero init fails (e.g. ort runtime unavailable), falls back to treating
/// the whole buffer as a single chunk — better than silently losing the
/// backfill audio.
fn vad_chunk_historical_audio(
    samples: &[f32],
    sample_rate: u32,
    tuning: &VadTuning,
) -> Vec<VadBackfillChunk> {
    if samples.is_empty() {
        return Vec::new();
    }

    let mut silero = match SileroVad::new() {
        Ok(v) => v,
        Err(e) => {
            warn!("backfill: Silero init failed ({e}) — single-chunk fallback");
            return vec![VadBackfillChunk {
                start: 0,
                end: samples.len(),
            }];
        }
    };

    let probabilities = silero.score_all(samples, sample_rate);
    if probabilities.is_empty() {
        return Vec::new();
    }

    backfill_chunks_from_probabilities(&probabilities, samples.len(), sample_rate, tuning)
}

/// Pure state-machine half of `vad_chunk_historical_audio`: walk a
/// pre-computed Silero probability stream and produce the same chunk
/// boundaries the live loop would have picked. Extracted so tests can
/// exercise the state machine with hand-crafted probability sequences
/// without loading the ONNX model.
fn backfill_chunks_from_probabilities(
    probabilities: &[f32],
    total_samples: usize,
    sample_rate: u32,
    tuning: &VadTuning,
) -> Vec<VadBackfillChunk> {
    if probabilities.is_empty() || total_samples == 0 {
        return Vec::new();
    }
    // One probability covers FRAME_DURATION_SECS (32 ms) of audio. Map
    // each frame back to the corresponding slice in the *original*
    // (un-resampled) buffer so chunk boundaries are expressed in the
    // caller's coordinate system.
    let frame_samples_original =
        (super::silero_vad::FRAME_DURATION_SECS * sample_rate as f32) as usize;
    if frame_samples_original == 0 {
        return Vec::new();
    }

    let onset_threshold = tuning.speech_threshold;
    let offset_threshold = tuning.speech_threshold * tuning.offset_threshold_ratio;
    let silence_windows = (tuning.silence_duration.as_secs_f32()
        / super::silero_vad::FRAME_DURATION_SECS)
        .ceil() as usize;
    let max_chunk_windows = (tuning.max_chunk_duration.as_secs_f32()
        / super::silero_vad::FRAME_DURATION_SECS)
        .max(1.0) as usize;
    let pre_roll_samples = (tuning.pre_roll.as_secs_f32() * sample_rate as f32) as usize;

    let mut chunks: Vec<VadBackfillChunk> = Vec::new();
    let mut is_speaking = false;
    let mut speech_start: usize = 0;
    let mut silence_run: usize = 0;
    let mut speech_run: usize = 0;
    // Lower bound for onset pre-roll — mirrors the live loop's
    // `earliest_next_chunk_pos` clamp in `poll_vad`. Without this, two
    // utterances separated by less than `pre_roll` would overlap: the
    // second onset would rewind into the first chunk's tail and the
    // transcriber would see the same audio twice.
    let mut prev_chunk_end: usize = 0;

    for (frame_idx, &prob) in probabilities.iter().enumerate() {
        let w_start = frame_idx * frame_samples_original;
        let w_end = ((frame_idx + 1) * frame_samples_original).min(total_samples);
        if w_start >= total_samples {
            break;
        }

        if is_speaking {
            speech_run += 1;
            let is_loud = prob >= offset_threshold;
            if is_loud {
                silence_run = 0;
                if speech_run >= max_chunk_windows {
                    chunks.push(VadBackfillChunk {
                        start: speech_start,
                        end: w_end,
                    });
                    prev_chunk_end = w_end;
                    speech_start = w_end;
                    speech_run = 0;
                }
            } else {
                silence_run += 1;
                if silence_run >= silence_windows {
                    chunks.push(VadBackfillChunk {
                        start: speech_start,
                        end: w_end,
                    });
                    prev_chunk_end = w_end;
                    is_speaking = false;
                    speech_run = 0;
                    silence_run = 0;
                }
            }
        } else {
            let is_loud = prob >= onset_threshold;
            if is_loud {
                speech_start = w_start.saturating_sub(pre_roll_samples).max(prev_chunk_end);
                is_speaking = true;
                speech_run = 1;
                silence_run = 0;
            }
        }
    }

    // Trailing speech that never resolved into silence — emit as final chunk
    // so we don't lose the tail of the backfill window.
    if is_speaking && speech_start < total_samples {
        chunks.push(VadBackfillChunk {
            start: speech_start,
            end: total_samples,
        });
    }

    chunks
}

/// Chunk audio into segments of approximately `chunk_size` samples, but refine
/// boundaries by scanning backward from each split point to find a silence gap.
/// This avoids splitting mid-word. Falls back to the fixed boundary if no silence
/// gap is found within the last `search_window` samples.
#[allow(dead_code)] // retained as fallback; backfill now uses vad_chunk_historical_audio
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
    cancel: Arc<AtomicBool>,
    tuning: VadTuning,
) {
    info!("backfill: starting concurrent processing");

    // Build per-source chunk lists using the live VAD state machine. This
    // gives backfill and live segmentation identical boundary choices —
    // no more quality gap between a session's first N seconds (historical)
    // and the rest (live). Uses the same engine-keyed `VadTuning`, so a
    // Parakeet backfill gets dialogue-tuned cadence + pre-roll.
    let mut source_entries: Vec<VadBackfillSource> = Vec::new();
    for (samples, sample_rate, label) in &backfill_audio {
        let chunks = vad_chunk_historical_audio(samples, *sample_rate, &tuning);
        debug!(
            "backfill: source={:?} {} chunks from {} samples ({:.1}s)",
            label,
            chunks.len(),
            samples.len(),
            samples.len() as f32 / *sample_rate as f32
        );
        source_entries.push(VadBackfillSource {
            label: *label,
            samples: samples.clone(),
            sample_rate: *sample_rate,
            chunks,
        });
    }

    // Per-source state
    let mut chunk_indices: Vec<u32> = vec![0; source_entries.len()];
    let mut accumulated_texts: Vec<String> = vec![String::new(); source_entries.len()];
    let session = Arc::new(StdMutex::new(SessionAccumulators {
        shared_prompt: String::new(),
        total_chunks: 0,
        total_audio_seconds: 0.0,
        last_transcription_at: None,
    }));

    // Interleave: process chunk 0 for all sources, then chunk 1, etc. Keeps
    // cross-source timestamps roughly interleaved instead of emitting all
    // of one source before the other.
    let total_chunks = source_entries
        .iter()
        .map(|s| s.chunks.len())
        .max()
        .unwrap_or(0);

    'outer: for chunk_idx in 0..total_chunks {
        if cancel.load(Ordering::Acquire) {
            info!(
                "backfill: cancel requested at chunk {} — exiting gracefully",
                chunk_idx
            );
            break 'outer;
        }
        for (source_idx, source) in source_entries.iter().enumerate() {
            if cancel.load(Ordering::Acquire) {
                break 'outer;
            }
            let Some(bounds) = source.chunks.get(chunk_idx) else {
                continue;
            };
            let slice = &source.samples[bounds.start..bounds.end];
            let audio_offset = bounds.start as f32 / source.sample_rate as f32;
            debug!(
                "backfill chunk: source={:?} idx={} offset={:.2}s samples={} ({:.2}s)",
                source.label,
                chunk_idx,
                audio_offset,
                slice.len(),
                slice.len() as f32 / source.sample_rate as f32
            );
            let input = ChunkInput {
                samples: slice,
                sample_rate: source.sample_rate,
                audio_offset_seconds: audio_offset,
                source_label: source.label,
                is_backfill: true,
            };
            transcribe_and_emit_chunk(
                &ctx,
                &input,
                &mut chunk_indices[source_idx],
                &mut accumulated_texts[source_idx],
                &session,
            )
            .await;
        }
    }

    {
        let s = session.lock().expect("session mutex poisoned");
        info!(
            "backfill: completed {} chunks, {:.1}s audio",
            s.total_chunks, s.total_audio_seconds
        );
    }

    // Bridge prompt context to live loop (Change 9: move instead of clone)
    {
        let mut prompt = ctx.bridged_prompt.lock().await;
        let max_prompt = ctx.config.prompt_context_chars.unwrap_or(350) as usize;
        let mut s = session.lock().expect("session mutex poisoned");
        if s.shared_prompt.len() > max_prompt {
            let boundary = s
                .shared_prompt
                .ceil_char_boundary(s.shared_prompt.len() - max_prompt);
            *prompt = s.shared_prompt[boundary..].to_string();
        } else {
            *prompt = std::mem::take(&mut s.shared_prompt);
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

    // Resolve engine-keyed VAD tuning once at loop entry. Whisper uses the
    // frontend-supplied silence_duration_ms / poll cadence; Parakeet uses its
    // dialogue-tuned defaults (400 ms silence, 100 ms poll, 250 ms pre-roll,
    // 0.7 offset-hysteresis ratio).
    let engine_kind = {
        let client_guard = ctx.transcription_client.lock().await;
        client_guard
            .as_ref()
            .map(|c| c.as_ref().engine())
            .unwrap_or(yapstack_common::types::EngineKind::Whisper)
    };
    let tuning = vad_tuning_for(engine_kind, &ctx.config);
    debug!(
        engine = engine_kind.as_str(),
        silence_ms = tuning.silence_duration.as_millis() as u64,
        poll_ms = tuning.poll_interval.as_millis() as u64,
        pre_roll_ms = tuning.pre_roll.as_millis() as u64,
        offset_ratio = tuning.offset_threshold_ratio,
        "live transcription VAD tuning resolved"
    );

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
                // Reset the full per-source live VAD state to `current`
                // so the live loop starts from the post-backfill write
                // position. Without this, Silero would replay every
                // backfill sample through the *live* detector on the
                // first few polls, duplicating or delaying speech that
                // backfill already emitted. We also reset the recurrent
                // stream state (LSTM memory was initialized at session
                // start against pre-backfill audio context) and clear the
                // sticky last_probability.
                s.cursor = current;
                s.speech_start_pos = current;
                s.earliest_next_chunk_pos = current;
                s.silero.read_pos = current;
                s.silero.last_probability = None;
                s.silero.stream.reset();
            }
        }

        (sources, backfill)
    };

    // Spawn concurrent backfill processing.
    // `backfill_cancel` is a cooperative cancel flag. On stop we set it true
    // and `process_backfill` finishes the in-flight chunk then exits cleanly
    // instead of being killed mid-transcribe (which would silently drop the
    // chunk's audio). We still keep an abort handle as a last-resort escape
    // hatch if the task hangs past our grace period.
    let backfill_done = Arc::new(AtomicBool::new(false));
    let backfill_cancel = Arc::new(AtomicBool::new(false));
    let backfill_handle = if !backfill_audio.is_empty() {
        // Namespace live chunk indices to avoid collision with backfill (0..N)
        for s in &mut sources {
            s.chunk_index = 10_000;
        }
        let backfill_ctx = ctx.clone();
        let backfill_done_clone = backfill_done.clone();
        let backfill_cancel_clone = backfill_cancel.clone();
        let handle = tokio::spawn(process_backfill(
            backfill_ctx,
            backfill_audio,
            backfill_done_clone,
            backfill_cancel_clone,
            tuning,
        ));
        let abort_handle = handle.abort_handle();
        Some((handle, abort_handle))
    } else {
        backfill_done.store(true, Ordering::Release);
        None
    };

    let session = Arc::new(StdMutex::new(SessionAccumulators {
        shared_prompt: String::new(),
        total_chunks: 0,
        total_audio_seconds: 0.0,
        last_transcription_at: None,
    }));
    let mut wav_flush_none_count: u32 = 0;
    let mut prompt_seeded_from_backfill = false;

    emit_status(&ctx.app_handle, LiveTranscriptionPhase::Running, 0, 0.0);

    let poll_interval = tuning.poll_interval;
    let mut exited_fatal = false;

    // Silero VAD runs in-process (bundled V5 ONNX). Single session shared
    // across sources; per-source streaming state lives on each VAD state's
    // `silero` field. `Session` is `Send` but not `Sync`, so we hold it
    // `mut` locally and feed sources sequentially inside each poll.
    let mut silero = match SileroVad::new() {
        Ok(v) => v,
        Err(e) => {
            error!(
                "live transcription: failed to initialize Silero VAD — bailing out: {}",
                e
            );
            emit_status(&ctx.app_handle, LiveTranscriptionPhase::Error, 0, 0.0);
            return;
        }
    };

    // In-flight chunk transcription tasks. `FuturesUnordered` so we can
    // wake the main loop the moment *any* task completes (via `.next()`
    // in the select!), rather than waiting for the next natural tick.
    // This cuts up to `poll_interval` of latency from the case where a
    // deferred `Chunk` action is waiting for a task to finish before it
    // can dispatch (e.g. a second utterance during the previous task's
    // transcribe). Outcomes drained here are applied at the top of the
    // tick below so VAD / dispatch see the updated per-source state.
    //
    // Each spawn is wrapped in an async block that traps `JoinError` and
    // synthesizes a fallback `ChunkTaskOutcome`. This guarantees the
    // invariant "every spawned task → exactly one outcome observed",
    // which keeps `has_in_flight_task` from leaking on panic/cancel and
    // would otherwise silently kill dispatch for that source forever.
    type ChunkTaskFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = ChunkTaskOutcome> + Send>>;
    let mut chunk_tasks: futures_util::stream::FuturesUnordered<ChunkTaskFuture> =
        futures_util::stream::FuturesUnordered::new();
    // Parallel handles so stop can `.abort()` slow tasks immediately —
    // otherwise an in-flight `transcribe_with` past the 10s drain would
    // hold an `Arc<TranscriptionClient>` clone, blocking try_unwrap and
    // leaving shared state empty (next session/dictation = NotInitialized).
    let mut chunk_aborts: Vec<tokio::task::AbortHandle> = Vec::new();
    let mut pending_outcomes: Vec<ChunkTaskOutcome> = Vec::new();

    loop {
        use futures_util::stream::StreamExt;
        use tokio::time::sleep;
        // Wait for the earliest of: a natural tick, a chunk task finishing,
        // or a stop signal. We recreate `sleep(poll_interval)` each
        // iteration so the next natural tick resets to `poll_interval` from
        // now — the tick cadence slides forward after an early wake, which
        // is fine here because the VAD state machine uses wall-clock
        // durations (Instant::now()), not tick count.
        let should_stop = tokio::select! {
            _ = sleep(poll_interval) => false,
            Some(outcome) = chunk_tasks.next(), if !chunk_tasks.is_empty() => {
                pending_outcomes.push(outcome);
                // Drain any other tasks that happen to be ready in this
                // same wakeup window without blocking.
                while let Some(Some(outcome)) = chunk_tasks.next().now_or_never() {
                    pending_outcomes.push(outcome);
                }
                false
            }
            _ = &mut stop_rx => true,
        };

        // Single lock: per-source audio extraction for Silero VAD +
        // WAV flush extraction. We pull raw mono samples (not just RMS)
        // for each source since the source's `silero.read_pos`, then feed
        // them through Silero outside the lock so the manager isn't held
        // while the ONNX session runs.
        struct SileroPollInput {
            label: AudioSourceLabel,
            samples: Option<(Vec<f32>, u32)>,
            new_pos: usize,
        }
        let (silero_inputs, wav_flush_data): (Vec<SileroPollInput>, _) = {
            let manager = audio_state.lock().await;
            let mut inputs: Vec<SileroPollInput> = Vec::with_capacity(sources.len());
            for s in &sources {
                let (samples, new_pos) =
                    extract_source_audio(&manager, &s.label, s.silero.read_pos);
                inputs.push(SileroPollInput {
                    label: s.label,
                    samples,
                    new_pos,
                });
            }
            let flush = session_wav_state.as_ref().and_then(|ws| {
                manager.extract_since(&ws.flush_positions, ws.source, ws.mix_config.as_ref())
            });
            (inputs, flush)
        };

        // Feed Silero outside the audio lock and collect *every* frame
        // probability per source, in emission order. A single poll batch
        // can span an entire short utterance; the VAD state machine needs
        // to see the intermediate loud frames, not just the trailing
        // silence frame that follows them.
        //
        // Sticky behavior: when no new samples arrived (empty buffer
        // snapshot), we carry the source's `last_probability` forward so
        // the state machine keeps making decisions on the most recent
        // signal rather than toggling to `None`.
        let mut per_source_probs: Vec<(AudioSourceLabel, Vec<f32>)> =
            Vec::with_capacity(silero_inputs.len());
        for input in silero_inputs {
            if let Some(s) = sources.iter_mut().find(|s| s.label == input.label) {
                s.silero.read_pos = input.new_pos;
                let probs = match input.samples {
                    Some((mono, source_sr)) => {
                        silero.score_stream(&mut s.silero.stream, &mono, source_sr)
                    }
                    None => Vec::new(),
                };
                if let Some(&last) = probs.last() {
                    s.silero.last_probability = Some(last);
                }
                per_source_probs.push((input.label, probs));
            }
        }

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
                {
                    let mut s = session.lock().expect("session mutex poisoned");
                    if s.shared_prompt.is_empty() {
                        s.shared_prompt = bridged.clone();
                    }
                    s.last_transcription_at = Some(Instant::now());
                }
                for source in &mut sources {
                    if source.accumulated_text.is_empty() {
                        source.accumulated_text = bridged.clone();
                    }
                }
                prompt_seeded_from_backfill = true;
                debug!(
                    "live loop: seeded prompt from backfill ({} chars)",
                    bridged.len()
                );
            }
        }

        // First: drain any completed chunk tasks (both the ones collected
        // by the `select!` wake above and any that finished during the
        // subsequent awaits) and apply their outcomes so their sources
        // become eligible for VAD polling again. We do this *before*
        // polling VAD so a just-completed task's source gets a fresh
        // poll_vad call (updated is_speaking / silence_since) this tick,
        // not next tick.
        let mut fatal = false;
        // Drain any additional outcomes that became ready while the tick
        // waited on other awaits (audio_state lock, WAV flush, etc). Also
        // prune finished abort handles to keep the vec bounded.
        while let Some(Some(outcome)) = chunk_tasks.next().now_or_never() {
            pending_outcomes.push(outcome);
        }
        chunk_aborts.retain(|h| !h.is_finished());
        let drained = std::mem::take(&mut pending_outcomes);
        for outcome in drained {
            for source in sources.iter_mut() {
                if source.label == outcome.source_label {
                    source.has_in_flight_task = false;
                    // Restore accumulated_text from the task. Safe because
                    // has_in_flight_task gated all reads of this field
                    // while the task was running.
                    source.accumulated_text = outcome.accumulated_text;
                    break;
                }
            }
            if outcome.sidecar_dead {
                fatal = true;
            }
        }

        // Poll VAD for *every* source, including those with an in-flight
        // task. The state machine needs to keep tracking is_speaking /
        // silence_since so a second utterance that starts and ends during
        // the in-flight window is captured: onset backdates
        // `speech_start_pos` (via pre_roll), silence_since fires a pending
        // Chunk action, and on the tick *after* the task lands we dispatch
        // that chunk with the full accumulated range.
        //
        // We feed *every* Silero probability from this batch through
        // `poll_vad` in order — intra-poll speech events would otherwise
        // be lost when only the trailing probability (likely silence) is
        // observed. If any frame produces a Chunk / ForceChunk action, we
        // remember it for the dispatch step below; at most one chunk can
        // actually fire per source per poll (has_in_flight_task gate), so
        // keeping the strongest observed action is the right summary.
        //
        // Dispatch and idle-cursor advance are still gated on
        // `has_in_flight_task`: we can't dispatch a second task for the
        // same source, and `advance_idle_cursor` would drop
        // `speech_start_pos` past audio that belongs to the next chunk.
        let actions: Vec<VadAction> = sources
            .iter_mut()
            .map(|source| {
                let probs = per_source_probs
                    .iter()
                    .find(|(l, _)| *l == source.label)
                    .map(|(_, p)| p.as_slice())
                    .unwrap_or(&[]);

                let mut summary = VadAction::None;
                if probs.is_empty() {
                    // Sticky fallback: no new frames in this batch. Use
                    // last_probability so the state machine keeps running
                    // against the most recent signal rather than freezing.
                    let action = poll_vad(source, source.silero.last_probability, &tuning);
                    if matches!(action, VadAction::Chunk | VadAction::ForceChunk) {
                        summary = action;
                    }
                } else {
                    for &prob in probs {
                        let action = poll_vad(source, Some(prob), &tuning);
                        match action {
                            VadAction::ForceChunk => summary = VadAction::ForceChunk,
                            VadAction::Chunk if !matches!(summary, VadAction::ForceChunk) => {
                                summary = VadAction::Chunk;
                            }
                            _ => {}
                        }
                    }
                }

                if should_stop && source.is_speaking {
                    summary = VadAction::Chunk;
                }
                summary
            })
            .collect();

        // Dispatch new chunk tasks (fire and forget). Per-source state is
        // advanced optimistically inside prepare_chunk_dispatch; the spawned
        // task handles transcription, segment emission, and quarantining on
        // failure. The main loop continues polling other sources next tick.
        for (i, action) in actions.iter().enumerate() {
            let in_flight = sources[i].has_in_flight_task;
            match action {
                VadAction::Chunk | VadAction::ForceChunk => {
                    if in_flight {
                        // Can't dispatch while the previous task is running.
                        // VAD state stays as poll_vad set it (is_speaking
                        // cleared, silence_since armed), so the next tick
                        // after the task lands will re-evaluate and fire a
                        // fresh Chunk from the accumulated speech_start_pos
                        // through the current write_pos.
                        continue;
                    }
                    let is_force = matches!(action, VadAction::ForceChunk);
                    let prepared =
                        prepare_chunk_dispatch(&mut sources[i], &audio_state, is_force, false)
                            .await;
                    if let Some(prepared) = prepared {
                        let task_ctx = ctx.clone();
                        let task_session = session.clone();
                        let source_label = prepared.source_label;
                        let fallback_text = prepared.accumulated_text.clone();
                        let handle = tokio::spawn(async move {
                            run_chunk_task(prepared, task_ctx, task_session).await
                        });
                        chunk_aborts.push(handle.abort_handle());
                        chunk_tasks.push(Box::pin(async move {
                            match handle.await {
                                Ok(outcome) => outcome,
                                Err(e) => {
                                    error!(
                                        "chunk task panicked or was cancelled for source {:?}: {} \
                                         — synthesizing outcome to free dispatch",
                                        source_label, e
                                    );
                                    ChunkTaskOutcome {
                                        source_label,
                                        accumulated_text: fallback_text,
                                        sidecar_dead: false,
                                    }
                                }
                            }
                        }));
                    }
                }
                VadAction::None => {
                    // Only advance the idle cursor when there's no in-flight
                    // task for this source AND we're confident no speech is
                    // pending. `has_in_flight_task` means `speech_start_pos`
                    // must stay frozen at the task's new_pos so any utterance
                    // that happened during the in-flight window still gets
                    // covered by the next chunk dispatch.
                    if !in_flight && !sources[i].is_speaking {
                        advance_idle_cursor(&mut sources[i], &audio_state).await;
                    }
                }
            }
        }

        if fatal {
            error!("live transcription: sidecar died and could not be restarted — stopping");
            let (chunks, audio_secs) = {
                let s = session.lock().expect("session mutex poisoned");
                (s.total_chunks, s.total_audio_seconds)
            };
            let _ = ctx.app_handle.emit(
                "live-transcription-status",
                LiveTranscriptionStatus {
                    phase: LiveTranscriptionPhase::Error,
                    chunks_processed: chunks,
                    total_audio_seconds: audio_secs,
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
        let last_at = {
            let s = session.lock().expect("session mutex poisoned");
            s.last_transcription_at
        };
        let decayed = {
            let mut s = session.lock().expect("session mutex poisoned");
            check_prompt_decay(
                &mut sources,
                &mut s.shared_prompt,
                prompt_decay_secs,
                last_at,
            )
        };
        if decayed {
            info!(
                "prompt decay: cleared shared_prompt ({:.1}s since last transcription)",
                last_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0)
            );
            session
                .lock()
                .expect("session mutex poisoned")
                .last_transcription_at = None;
        }

        if should_stop {
            break;
        }
    }

    // Drain any still-running chunk tasks so their segments land before we
    // finalize the session. The `setLivePhase("Stopped")` finalizer on the
    // frontend awaits `segmentQueueTail`, but for that to help we first
    // need to actually *dispatch* the segment events — which only happens
    // when each task's transcribe await completes.
    if !chunk_tasks.is_empty() {
        debug!(
            "live transcription stop: draining {} in-flight chunk tasks",
            chunk_tasks.len()
        );
        // Cancel everything still running so the drain completes promptly
        // and each task's `Arc<TranscriptionClient>` clone is released —
        // otherwise the post-loop try_unwrap fails and shared state is
        // left empty, which surfaces as "transcription client not
        // initialized" on the next session/dictation. transcribe_with
        // awaits a oneshot internally; abort drops the receiver cleanly,
        // and the sidecar's eventual response is harmlessly orphaned by
        // the reader task.
        for ah in chunk_aborts.drain(..) {
            ah.abort();
        }
        // Bounded wait so a hanging sidecar can't indefinitely block the
        // stop path. With abort above, tasks normally complete in <1s.
        // The per-task timeout inside transcribe_with is 300s; the 10s
        // outer cap stays as a belt-and-suspenders measure.
        use futures_util::stream::StreamExt;
        let drain = async {
            while let Some(outcome) = chunk_tasks.next().await {
                if outcome.sidecar_dead {
                    warn!("drained chunk task reported sidecar dead");
                }
            }
        };
        if tokio::time::timeout(Duration::from_secs(10), drain)
            .await
            .is_err()
        {
            warn!(
                "live transcription stop: chunk task drain timed out after 10s; \
                 any unfinished chunks will be dropped"
            );
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
            // Finalize WAV only (no MP3 conversion) to release file handle, then delete
            let _ = ws.writer.finalize_wav_only();
            let _ = std::fs::remove_file(&wav_path);
            let _ = ctx.app_handle.emit(
                "session-wav-error",
                SessionWavErrorEvent {
                    session_id: ws.session_id,
                    message: "No audio was recorded — WAV file not saved".to_string(),
                },
            );
        } else {
            let use_mp3 = ctx.config.audio_export_format.as_deref().unwrap_or("mp3") != "wav";
            let result = if use_mp3 {
                ws.writer
                    .finalize_as_mp3(ctx.config.mp3_bitrate.unwrap_or(64))
            } else {
                ws.writer.finalize_wav_only()
            };
            match result {
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

    // Wait for concurrent backfill to finish before emitting Stopped.
    // Signal the cooperative cancel flag so the task exits at the next window
    // boundary instead of running to completion when the session is stopping.
    // The abort handle is a last-resort escape hatch if the in-flight chunk
    // hangs past the grace period.
    if let Some((handle, abort_handle)) = backfill_handle {
        backfill_cancel.store(true, Ordering::Release);
        match tokio::time::timeout(Duration::from_secs(30), handle).await {
            Ok(_) => {}
            Err(_) => {
                warn!("backfill task did not exit within 30s after cancel — aborting");
                abort_handle.abort();
            }
        }
    }

    let (final_chunks, final_audio_seconds) = {
        let s = session.lock().expect("session mutex poisoned");
        (s.total_chunks, s.total_audio_seconds)
    };

    // Only emit Stopped if we didn't already emit Error (avoids duplicate finalization)
    if !exited_fatal {
        emit_status(
            &ctx.app_handle,
            LiveTranscriptionPhase::Stopped,
            final_chunks,
            final_audio_seconds,
        );
    }

    info!(
        "live transcription stopped: {} chunks, {:.1}s total audio",
        final_chunks, final_audio_seconds
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
    session: &Arc<StdMutex<SessionAccumulators>>,
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

    // Briefly lock the outer mutex just to clone the inner Arc<TranscriptionClient>.
    // The actual `transcribe_with` await happens *without* holding any outer lock,
    // so a concurrent chunk task (e.g. the other source) can proceed in parallel.
    let client_arc = {
        let client_guard = ctx.transcription_client.lock().await;
        client_guard.as_ref().cloned()
    };
    let client = match client_arc {
        Some(c) => c,
        None => {
            error!("live transcription: transcription client not initialized");
            let _ = ctx.app_handle.emit(
                "live-transcription-status",
                LiveTranscriptionStatus {
                    phase: LiveTranscriptionPhase::Error,
                    chunks_processed: *chunk_index,
                    total_audio_seconds: 0.0,
                    error_message: Some("transcription client not initialized".to_string()),
                    session_id: ctx.config.session_id.clone(),
                    effective_start_epoch_ms: None,
                },
            );
            return TranscribeOutcome::SidecarDead;
        }
    };
    let engine_kind = client.engine();
    // initial_prompt is Whisper-only — Parakeet's TDT decoder has no text-prompt
    // input, so passing it would just be ignored. Drop it explicitly so the IPC
    // payload is honest about what was sent.
    let prompt_for_engine = match engine_kind {
        yapstack_common::types::EngineKind::Whisper => effective_prompt,
        yapstack_common::types::EngineKind::Parakeet => None,
    };
    let transcription_result = client
        .transcribe_with(
            &wav_path,
            ctx.config.language.as_deref(),
            prompt_for_engine,
            ctx.config.diarization,
        )
        .await;

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
                .filter(|t| !yapstack_common::hallucination::is_always_reject(t, engine_kind))
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
                let max_prompt = ctx.config.prompt_context_chars.unwrap_or(350) as usize;
                let mut s = session.lock().expect("session mutex poisoned");
                if !s.shared_prompt.is_empty() {
                    s.shared_prompt.push(' ');
                }
                s.shared_prompt.push_str(&prompt_text);
                if s.shared_prompt.len() > max_prompt {
                    let boundary = s
                        .shared_prompt
                        .ceil_char_boundary(s.shared_prompt.len() - max_prompt);
                    s.shared_prompt = s.shared_prompt[boundary..].to_string();
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
                    speaker_id: s.speaker_id,
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
                    session_id: ctx.config.session_id.clone(),
                },
                chunk_duration,
            })
        }
        Err(e) => {
            warn!("live transcription: chunk failed: {}, skipping", e);

            // Drop our local Arc clone before attempting restart so the outer
            // mutex holder can try_unwrap without racing us. `client` (the
            // Arc we cloned above) went out of scope at the end of the Ok
            // arm's destructor, but we still hold our `client_arc` binding
            // from the transcribe section. Drop it explicitly.
            drop(client);

            // Check if the sidecar process died — attempt auto-restart.
            // respawn() needs &mut TranscriptionClient, so we take the Arc
            // out of the Option, try_unwrap it, respawn, and put it back.
            // If another task is still holding an Arc clone (e.g. a
            // concurrent chunk still awaiting a response from the dead
            // sidecar), try_unwrap fails and we fall through to Skipped —
            // the other task will hit the same error and one of us will
            // eventually win the race.
            let mut client_guard = ctx.transcription_client.lock().await;
            if let Some(arc_client) = client_guard.take() {
                if !arc_client.is_running() {
                    warn!("sidecar process died — attempting restart");
                    match Arc::try_unwrap(arc_client) {
                        Ok(mut client) => {
                            match client.respawn().await {
                                Ok(()) => {
                                    info!(
                                        "sidecar restarted successfully after transcription failure"
                                    );
                                    *client_guard = Some(Arc::new(client));
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
                                    // Put the client back so other tasks can
                                    // see it as "not running" and try again.
                                    *client_guard = Some(Arc::new(client));
                                    return TranscribeOutcome::SidecarDead;
                                }
                            }
                        }
                        Err(still_shared) => {
                            // Another task still holds the Arc. Put it back
                            // untouched; that task will hit the same error
                            // and we'll try again on the next chunk.
                            *client_guard = Some(still_shared);
                            debug!(
                                "sidecar restart skipped: client still held by another chunk task"
                            );
                            return TranscribeOutcome::Skipped;
                        }
                    }
                }
                // Sidecar still running — just a transient error. Put the
                // Arc back and let the caller retry.
                *client_guard = Some(arc_client);
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
    session: &Arc<StdMutex<SessionAccumulators>>,
) -> TranscribeOutcome {
    let outcome = transcribe_chunk(ctx, input, chunk_index, accumulated_text, session).await;

    if let TranscribeOutcome::Success(ref result) = outcome {
        // Short critical section — no await held.
        let (total_chunks, total_audio_seconds) = {
            let mut s = session.lock().expect("session mutex poisoned");
            s.total_chunks += 1;
            s.total_audio_seconds += result.chunk_duration;
            s.last_transcription_at = Some(Instant::now());
            (s.total_chunks, s.total_audio_seconds)
        };

        let _ = ctx
            .app_handle
            .emit("live-transcription-segment", &result.event);
        emit_status(
            &ctx.app_handle,
            LiveTranscriptionPhase::Running,
            total_chunks,
            total_audio_seconds,
        );
    }

    outcome
}

// --- Tauri commands ---

#[tauri::command]
#[specta::specta]
pub async fn start_live_transcription(
    audio_state: tauri::State<'_, AudioManagerState>,
    transcription_state: tauri::State<'_, TranscriptionClientState>,
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
    let transcription_state_clone = transcription_state.inner().clone();

    // Align config backfill with the clamped value so WAV writer and transcript
    // cursor share the same time origin (prevents timestamp drift on playback).
    config.backfill_seconds = effective_backfill_seconds;

    // Capture session_id before config is moved into TranscriptionContext
    let controller_session_id = config.session_id.clone();

    // Extract the transcription client only after all fallible setup above
    // succeeds. This avoids losing the client on early-return setup errors.
    let extracted_client = {
        let mut client_guard = transcription_state.lock().await;
        client_guard.take().ok_or(CommandError::NotInitialized {
            message: "transcription client not initialized".into(),
        })?
    };

    let ctx = TranscriptionContext {
        transcription_client: Arc::new(Mutex::new(Some(Arc::new(extracted_client)))),
        shared_transcription_state: transcription_state_clone,
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

            // Always return the transcription client to shared state, even
            // after a panic. The client is held as `Arc<TranscriptionClient>`
            // during the loop so concurrent chunk tasks can each hold a ref;
            // by the time we get here, the loop has joined and we should be
            // the last Arc holder. If try_unwrap fails it means a task is
            // leaking a clone — log loudly and drop, which is still safe
            // (the Drop impl kills the sidecar process).
            {
                let mut private_guard = ctx_guard.transcription_client.lock().await;
                if let Some(arc_client) = private_guard.take() {
                    match Arc::try_unwrap(arc_client) {
                        Ok(client) => {
                            let mut shared_guard =
                                ctx_guard.shared_transcription_state.lock().await;
                            *shared_guard = Some(client);
                            debug!("returned transcription client to shared state");
                        }
                        Err(_still_shared) => {
                            error!(
                                "live transcription ended but a task still holds the \
                                 transcription client — shared state left empty; \
                                 subsequent sessions and dictation will fail with \
                                 NotInitialized until the app restarts. This means a \
                                 chunk task did not respond to abort; please report."
                            );
                        }
                    }
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

    /// Shorthand for poll_vad under the Silero-based state machine.
    /// `energy` here is now a *Silero speech probability* in [0, 1], not
    /// an RMS value. Test assertions that previously used 0.005 / 0.05
    /// RMS values are paired with `scale_probability` below which maps
    /// them onto probabilities around the threshold.
    fn poll(state: &mut SourceVadState, probability: Option<f32>) -> VadAction {
        let tuning = VadTuning {
            speech_threshold: 0.5,
            offset_threshold_ratio: 1.0,
            silence_duration: Duration::from_millis(800),
            max_chunk_duration: Duration::from_secs(30),
            poll_interval: Duration::from_millis(300),
            pre_roll: Duration::ZERO,
        };
        poll_vad(state, probability, &tuning)
    }

    #[test]
    fn test_poll_vad_silence_returns_none() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        // Below threshold while not speaking → None, stays not-speaking
        let action = poll(&mut state, Some(0.10));
        assert!(matches!(action, VadAction::None));
        assert!(!state.is_speaking);
    }

    #[test]
    fn test_poll_vad_speech_onset() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        // Above threshold while not speaking → transitions to speaking
        let action = poll(&mut state, Some(0.90));
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
        let action = poll(&mut state, Some(0.90));
        assert!(matches!(action, VadAction::None));
        assert!(state.silence_since.is_none());
    }

    #[test]
    fn test_poll_vad_energy_at_exact_threshold() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        // Exactly at threshold (0.50) while not speaking — uses >=, so SHOULD trigger onset
        let action = poll(&mut state, Some(0.50));
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
        let action = poll(&mut state, Some(0.10));
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
        let action = poll(&mut state, Some(0.10));
        assert!(matches!(action, VadAction::Chunk));
    }

    #[test]
    fn test_poll_vad_force_chunk_max_duration() {
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);
        state.is_speaking = true;
        state.speech_start_time = Some(Instant::now() - Duration::from_secs(31));
        // Above threshold while speaking past max_chunk_duration → ForceChunk
        let action = poll(&mut state, Some(0.90));
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

    // --- VAD tuning guard tests ---

    fn dummy_config() -> LiveTranscriptionConfig {
        LiveTranscriptionConfig {
            silence_threshold: 0.01,
            silence_duration_ms: 800,
            max_chunk_seconds: 30.0,
            backfill_seconds: 0.0,
            source: CaptureSourceDto::MicOnly,
            mix_config: None,
            language: None,
            prompt_context_chars: None,
            prompt_decay_silence_seconds: None,
            session_id: None,
            audio_save_location: None,
            audio_export_format: None,
            mp3_bitrate: None,
            diarization: false,
        }
    }

    #[test]
    fn test_vad_tuning_whisper_honors_user_silence_duration() {
        // Whisper still honors the user's frontend silence_duration_ms
        // and max_chunk_seconds — only the detector (RMS → Silero) swaps.
        // Threshold values are now Silero probabilities (0.5 / 0.35),
        // shared across engines.
        use yapstack_common::types::EngineKind;
        let mut cfg = dummy_config();
        cfg.silence_duration_ms = 600;
        cfg.max_chunk_seconds = 25.0;
        let t = vad_tuning_for(EngineKind::Whisper, &cfg);
        assert_eq!(t.silence_duration, Duration::from_millis(600));
        assert_eq!(t.max_chunk_duration, Duration::from_secs_f32(25.0));
        assert!((t.speech_threshold - 0.5).abs() < 1e-6);
        // 0.35 / 0.5 = 0.7 hysteresis ratio
        assert!((t.offset_threshold_ratio - 0.7).abs() < 1e-6);
        assert_eq!(t.pre_roll, Duration::ZERO);
    }

    #[test]
    fn test_vad_tuning_parakeet_ignores_user_knobs() {
        // Parakeet tuning is engine-specific backend policy — even if the
        // user sets silence_duration_ms=800 and max_chunk_seconds=30 in the
        // frontend, Parakeet uses its dialogue-aggressive defaults.
        use yapstack_common::types::EngineKind;
        let cfg = dummy_config();
        let t = vad_tuning_for(EngineKind::Parakeet, &cfg);
        assert_eq!(
            t.silence_duration,
            Duration::from_millis(200),
            "Parakeet silence window should be 200ms for responsive splitting"
        );
        assert_eq!(
            t.max_chunk_duration,
            Duration::from_secs(10),
            "Parakeet should force-chunk at 10s of continuous speech"
        );
        assert_eq!(t.poll_interval, Duration::from_millis(100));
        assert_eq!(t.pre_roll, Duration::from_millis(250));
        assert!((t.speech_threshold - 0.5).abs() < 1e-6);
        assert!((t.offset_threshold_ratio - 0.7).abs() < 1e-6);
    }

    // --- Review regression tests ---

    /// Regression: intra-poll speech detection.
    ///
    /// When a poll batch contains speech followed by silence, the VAD state
    /// machine must enter speaking state — not be fooled by only the last
    /// frame's silence probability. This mirrors the live-loop code path
    /// where we now iterate every Silero probability through `poll_vad`.
    #[test]
    fn poll_vad_enters_speaking_within_a_single_poll_batch() {
        use yapstack_common::types::EngineKind;
        let cfg = dummy_config();
        let tuning = vad_tuning_for(EngineKind::Parakeet, &cfg);
        let mut state = SourceVadState::new(AudioSourceLabel::Mic, 0, 0, 48000, 1);

        // Simulate one 100 ms Parakeet poll containing: silence → loud →
        // silence. Previously (score_stream returning only the last prob),
        // the live loop would have seen only the trailing silence and the
        // speech onset would have been dropped.
        let probs = [0.10f32, 0.90, 0.10];
        for &p in &probs {
            let _ = poll_vad(&mut state, Some(p), &tuning);
        }

        assert!(
            state.is_speaking,
            "state should have transitioned to speaking during the batch"
        );
        assert!(state.speech_start_time.is_some());
        // The trailing silence frame starts the silence timer but doesn't
        // itself fire Chunk (needs `silence_duration` = 200 ms; only one
        // frame elapsed).
        assert!(state.silence_since.is_some());
    }

    /// Regression: backfill pre-roll must not rewind into a previous chunk.
    ///
    /// Two loud regions separated by a silence gap shorter than
    /// `pre_roll_samples` used to produce overlapping chunks — the second
    /// onset rewound past the first chunk's end. With the
    /// `prev_chunk_end` clamp this no longer happens.
    ///
    /// Exercises the pure state-machine half of the backfill chunker
    /// against a hand-crafted probability sequence so the test isn't
    /// dependent on Silero classifying a synthetic tone as speech.
    #[test]
    fn backfill_chunks_do_not_overlap_when_gap_shorter_than_pre_roll() {
        use yapstack_common::types::EngineKind;
        let cfg = dummy_config();
        let tuning = vad_tuning_for(EngineKind::Parakeet, &cfg);
        assert_eq!(tuning.pre_roll, Duration::from_millis(250));
        assert_eq!(tuning.silence_duration, Duration::from_millis(200));

        let sr: u32 = 48_000;
        // At 32 ms per frame: 1.0 s = ~31 frames, 0.3 s = ~9 frames.
        let loud_frames = 31;
        let gap_frames = 9;
        // Shape: [loud × 31][silence × 9][loud × 31]. The silence stretch
        // (288 ms) crosses the 200 ms silence_duration so the first chunk
        // fires; the gap (288 ms < 250 ms pre-roll + ~32 ms frame) means
        // without the `prev_chunk_end` clamp, the second onset's pre-roll
        // would reach back into the first chunk's tail.
        let mut probs: Vec<f32> = Vec::with_capacity(loud_frames * 2 + gap_frames);
        probs.extend(std::iter::repeat(0.90).take(loud_frames));
        probs.extend(std::iter::repeat(0.10).take(gap_frames));
        probs.extend(std::iter::repeat(0.90).take(loud_frames));

        let frame_samples = (super::super::silero_vad::FRAME_DURATION_SECS * sr as f32) as usize;
        let total_samples = probs.len() * frame_samples;

        let chunks = backfill_chunks_from_probabilities(&probs, total_samples, sr, &tuning);
        assert!(
            chunks.len() >= 2,
            "expected at least 2 chunks for two loud regions, got {}: {:?}",
            chunks.len(),
            chunks
        );
        for w in chunks.windows(2) {
            assert!(
                w[1].start >= w[0].end,
                "chunks overlap: first ends at {}, next starts at {}",
                w[0].end,
                w[1].start
            );
        }
    }

    /// Regression: after backfill extraction, the live VAD must start from
    /// the current write position — not the rewound backfill cursor — so
    /// the live Silero reader doesn't replay backfill samples.
    ///
    /// This is a structural check: we simulate the reset block in
    /// `live_transcription_loop` against a `SourceVadState` built with a
    /// rewound initial position, then verify the fields all advance to
    /// the post-backfill `current` position.
    #[test]
    fn silero_state_resets_to_current_position_after_backfill_extract() {
        let session_start: usize = 1_000; // rewound backfill position
        let current: usize = 50_000; // write_pos after capturing 1s @ 48k mono

        let mut s = SourceVadState::new(
            AudioSourceLabel::Mic,
            session_start,
            session_start,
            48000,
            1,
        );
        // Pre-reset invariants: everything points at the rewound position.
        assert_eq!(s.silero.read_pos, session_start);
        assert_eq!(s.speech_start_pos, session_start);
        assert_eq!(s.earliest_next_chunk_pos, session_start);

        // Apply the same reset the live loop does after backfill extract.
        s.cursor = current;
        s.speech_start_pos = current;
        s.earliest_next_chunk_pos = current;
        s.silero.read_pos = current;
        s.silero.last_probability = None;
        s.silero.stream.reset();

        assert_eq!(
            s.silero.read_pos, current,
            "Silero read cursor must jump past the backfill window"
        );
        assert_eq!(s.cursor, current);
        assert_eq!(s.speech_start_pos, current);
        assert_eq!(s.earliest_next_chunk_pos, current);
        assert!(s.silero.last_probability.is_none());
    }
}
