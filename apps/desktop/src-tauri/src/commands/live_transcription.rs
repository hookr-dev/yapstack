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
    /// Session ID for streaming session-audio recording. If set, the loop
    /// streams a new audio part to disk during the session and emits
    /// `session-part-ready` at finalize time. The on-disk file is
    /// `{audio_save_location || $APP_DATA_DIR/audio/}/{session_id}.{part_index}.{wav|mp3}`,
    /// where `part_index` is 0 for a fresh session and `N` when resuming a
    /// session that already has parts.
    pub session_id: Option<String>,
    /// When `true`, the finalize path inserts a `session_audio_parts` row
    /// keyed by `session_id` so the DB stays the durable source of truth even
    /// if the FE listener is gone. Set this to `false` for synthetic ids that
    /// are *not* rows in `sessions` — most importantly dictation, where the
    /// id is per-utterance and the finalized file is recorded against
    /// `dictation_history` instead. Defaults to `true` to preserve historical
    /// behavior for actual sessions; the dictation hook flips it off.
    #[serde(default = "default_persist_audio_part")]
    pub persist_audio_part: bool,
    /// Custom directory for saving session audio parts. If None, uses
    /// `$APP_DATA_DIR/audio/`. The directory is registered with
    /// `audio_save_locations` at recording start so reconciliation can
    /// recover orphan parts on the next startup.
    pub audio_save_location: Option<String>,
    /// Audio export format applied at part finalization. Default: `Mp3`
    /// (matches the legacy "no value provided" behaviour). Choosing `Mp3`
    /// re-encodes the streamed WAV at `mp3_bitrate` and deletes the source
    /// WAV; `Wav` keeps the streamed file as-is. Typed end-to-end so a stale
    /// or typo'd caller fails at deserialization rather than silently
    /// rerouting through the MP3 branch.
    pub audio_export_format: Option<AudioExportFormatDto>,
    /// MP3 bitrate in kbps (e.g. 64, 128, 192). Only used when format is "mp3".
    pub mp3_bitrate: Option<u16>,
    /// Request speaker diarization on every transcribed chunk. Honored only
    /// when the active engine is Parakeet *and* the sidecar was spawned with
    /// a Sortformer model path. Whisper sessions ignore this flag.
    #[serde(default)]
    pub diarization: bool,
    /// Comma-separated vocabulary hints (folder/tag names) prepended to the
    /// Whisper initial_prompt to improve recognition of proper nouns. Ignored
    /// for Parakeet sessions (the TDT decoder has no text-prompt input).
    pub vocabulary_hints: Option<String>,
    /// When set, this run is a resume of an existing Session. The new
    /// recording becomes a fresh audio part appended after the existing parts;
    /// no prior file is read or modified. Backfill is forced to 0.
    #[serde(default)]
    pub resume: Option<ResumeConfig>,
}

fn default_persist_audio_part() -> bool {
    true
}

/// Typed surface for the audio finalize format. Lowercase serde tags match
/// the legacy `"wav"` / `"mp3"` strings the FE already passes and the
/// `format` field on `SessionPartReadyEvent` already emits, so this is a
/// drop-in tightening of the Tauri boundary — generated TypeScript becomes
/// a `"wav" | "mp3"` discriminated union instead of `string | null`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum AudioExportFormatDto {
    Wav,
    Mp3,
}

impl AudioExportFormatDto {
    pub fn is_mp3(self) -> bool {
        matches!(self, AudioExportFormatDto::Mp3)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AudioExportFormatDto::Wav => "wav",
            AudioExportFormatDto::Mp3 => "mp3",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ResumeConfig {
    /// The index of the new part being recorded — equals the count of
    /// existing parts in the session. The output WAV/MP3 is named
    /// `{session_id}.{part_index}.{ext}`.
    pub part_index: u32,
    /// Cumulative duration of the existing parts. Added to every Segment's
    /// `audio_offset_seconds` so persisted Segments stay continuous.
    pub offset_base_seconds: f32,
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
    /// Session-time elapsed since session start minus the latest completed
    /// chunk's end offset. Rising values mean transcription is falling behind
    /// real time; falling values mean the consumer is catching up. None until
    /// the first successful chunk lands.
    pub lag_seconds: Option<f32>,
    /// Cumulative count of times the Stage-3 head-drop cap fired this
    /// session. Stays 0 in normal operation — any non-zero value indicates
    /// audio was discarded to keep the inference queue bounded. Removed in
    /// Stage 3 when cap-and-drop is replaced with queue-and-drain.
    pub cap_fired_total: u32,
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
pub struct SessionPartReadyEvent {
    pub session_id: String,
    pub part_index: u32,
    pub file_path: String,
    /// Always emitted as the typed enum (`"wav"` or `"mp3"`). Listeners can
    /// branch on this without string-literal guards.
    pub format: AudioExportFormatDto,
    pub duration_seconds: f32,
    pub sample_rate: u32,
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
    /// Human-readable name of the device the Stream is bound to after the
    /// event. Set on successful auto-failover so the FE can render
    /// "Switched to {name}" toasts. `None` for failures or when the
    /// underlying capture didn't report a device name.
    #[serde(default)]
    pub bound_device_name: Option<String>,
}

/// Per-chunk timing telemetry. Emitted after every transcribe attempt (success
/// OR failure) so the frontend and logs can see when the pipeline is falling
/// behind real time. `wall_ms / (chunk_audio_seconds * 1000) = RTFx`; values
/// below 1.0 mean the consumer is slower than real time and lag will grow.
#[derive(Debug, Clone, Serialize, Type)]
pub struct LiveTranscriptionPressureEvent {
    pub source: AudioSourceLabel,
    pub chunk_index: u32,
    pub chunk_audio_seconds: f32,
    pub wall_ms: u64,
    /// `chunk_audio_seconds / (wall_ms / 1000)`. None when transcribe failed
    /// or wall_ms is 0.
    pub rtfx: Option<f32>,
    /// Engine that produced this chunk — `"Whisper"` or `"Parakeet"`.
    pub engine: String,
    pub is_backfill: bool,
    /// Session-time elapsed since session start minus the just-completed
    /// chunk's end offset. Positive means "transcription is N seconds behind
    /// real time at the moment this chunk finished." None when the chunk did
    /// not produce a successful Transcription response.
    pub lag_seconds: Option<f32>,
    /// Resolved accelerator (`"webgpu"`, `"coreml"`, `"cuda"`, `"cpu"`,
    /// `"metal"`) for the active sidecar. Captured once at session start
    /// from the client's cached engine info — rises with the rest of the
    /// pressure payload so a single grep'd `live_pressure` line tells us
    /// whether a slow chunk happened on GPU or CPU.
    pub accel: Option<String>,
    /// For Parakeet, the variant directory name (e.g.
    /// `"parakeet-tdt-v3-int8"` or `"parakeet-tdt-v3"`) so we can tell
    /// int8 vs fp32 sessions apart in logs without joining against
    /// `live_engine_loaded`. None for Whisper (single bundle) or when
    /// the sidecar didn't report a model_dir.
    pub variant: Option<String>,
}

/// Internal state for streaming WAV recording during a live session.
struct SessionWavState {
    writer: yapstack_audio::SessionWavWriter,
    flush_positions: BufferPositions,
    source: CaptureSource,
    mix_config: Option<yapstack_audio::MixConfig>,
    session_id: String,
    /// Index of this part within its session (0 for fresh, N for resume).
    part_index: u32,
    flush_count: u32,
    /// Sample rate the WAV file's header was opened with. If a source's
    /// ring buffer gets replaced at a different sample rate mid-session
    /// (device format change), subsequent extracted samples are resampled
    /// to this rate before `write_samples` so the archived file plays at
    /// a single consistent speed.
    wav_sample_rate: u32,
    /// When `true`, finalize inserts a `session_audio_parts` row keyed by
    /// `session_id`. Set to `false` for dictation, whose `session_id` is a
    /// synthetic per-utterance value with no `sessions` row backing it.
    persist_audio_part: bool,
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
    /// Dynamic vocabulary hints updated by frontend during recording.
    vocabulary_hints: Arc<Mutex<String>>,
    /// Constant added to every Segment's `audio_offset_seconds` so resumed
    /// runs produce offsets continuous with prior parts. Zero on a fresh
    /// Session, equals SUM of prior part durations on a resumed Session.
    session_offset_base_seconds: f32,
    /// Wall-clock instant the live loop started. Combined with
    /// `session_offset_base_seconds` this gives "session-time now" for lag
    /// calculations: lag = (offset_base + start.elapsed()) - latest_completed_audio_offset.
    session_start_instant: Instant,
    /// Engine-conditional configuration resolved once when the session
    /// starts. Live loop, `transcribe_chunk`, and `run_prompt_decay`
    /// consult this instead of branching on `EngineKind` at every site.
    /// `Arc` so the cheap `Clone` on `TranscriptionContext` doesn't
    /// re-allocate the underlying tuning struct.
    engine_profile: Arc<EngineProfile>,
}

/// Result of transcribing a single chunk.
struct ChunkResult {
    event: LiveSegmentEvent,
    chunk_duration: f32,
    /// Wall-time the sidecar took to process this chunk (round-trip
    /// `transcribe_with` duration). Tracked here so the success path can
    /// fold it into `SessionCounters::total_wall_ms` without re-measuring.
    wall_ms: u64,
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

/// Cross-loop chunk counters surfaced to `LiveTranscriptionController` for
/// status reporting. Shared between the live loop and `process_backfill` so
/// `chunks_processed` / `total_audio_seconds` reflect both sources of work.
///
/// All mutation sites lock briefly; the mutex is never held across an await.
struct SessionCounters {
    total_chunks: u32,
    total_audio_seconds: f32,
    /// Cumulative count of head-drop cap firings (Stage 3 will remove the
    /// cap; for now this counts how often we silently discarded audio).
    cap_fired_total: u32,
    /// Cumulative wall-time across every successful transcribe — for an
    /// aggregate "session RTFx" view if we want one later.
    total_wall_ms: u64,
    /// Session-time of the latest completed chunk's end (audio_offset +
    /// chunk_duration). Used by `get_live_transcription_status` to compute
    /// lag against the current wall clock. None until the first chunk lands.
    latest_completed_audio_offset_seconds: Option<f32>,
}

/// Per-loop prompt accumulator. The live loop and `process_backfill` each own
/// their own instance — they must not share, because backfill bridges its
/// final accumulated prompt out to `bridged_prompt`, and a concurrent live
/// chunk writing into the same buffer would either get clobbered by the
/// bridge or cause the bridge to ship live text back into `bridged_prompt`.
///
/// Shared among concurrent per-source live chunk tasks via `Arc<StdMutex<_>>`.
/// All mutation sites lock briefly; the mutex is never held across an await.
struct PromptState {
    shared_prompt: String,
    /// When the last successful transcription occurred. Used for prompt decay —
    /// if no transcription has happened for `prompt_decay_silence_seconds`, all
    /// prompt context is cleared to prevent stale text from causing hallucinations.
    last_transcription_at: Option<Instant>,
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

pub(crate) struct LiveTranscriptionController {
    task_handle: tokio::task::JoinHandle<()>,
    stop_tx: Option<oneshot::Sender<()>>,
    session_id: Option<String>,
    effective_start_epoch_ms: f64,
    /// Live counters shared with the running loop and backfill task. They
    /// mutate in place when a chunk lands; `get_live_transcription_status`
    /// reads them via a brief lock to surface real values to the UI.
    counters: Arc<StdMutex<SessionCounters>>,
    /// Mirrors `TranscriptionContext::session_start_instant` so the polled
    /// status command can compute `lag_seconds` without having to reach into
    /// the running loop's private state.
    session_start_instant: Instant,
    /// Mirrors `TranscriptionContext::session_offset_base_seconds` for the
    /// same reason — needed to convert wall-clock elapsed into session-time.
    session_offset_base_seconds: f32,
}

impl LiveTranscriptionController {
    pub fn is_running(&self) -> bool {
        !self.task_handle.is_finished()
    }

    /// Snapshot the live counters and derive `lag_seconds` against the wall
    /// clock under a single short lock. `lag_seconds` is `None` until the
    /// first successful chunk has landed.
    fn snapshot(&self) -> (u32, f32, Option<f32>, u32) {
        let s = self.counters.lock().expect("counters mutex poisoned");
        let lag = s.latest_completed_audio_offset_seconds.map(|chunk_end| {
            let session_time_now = self.session_offset_base_seconds
                + self.session_start_instant.elapsed().as_secs_f32();
            (session_time_now - chunk_end).max(0.0)
        });
        (
            s.total_chunks,
            s.total_audio_seconds,
            lag,
            s.cap_fired_total,
        )
    }
}

pub struct LiveTranscriptionRuntime {
    pub(crate) controller: LiveTranscriptionController,
    vocabulary_hints: Arc<Mutex<String>>,
}

pub type LiveTranscriptionState = Arc<Mutex<Option<LiveTranscriptionRuntime>>>;

/// Inbox for cross-thread "please restart this Source" requests, used by
/// the device broker (`device_broker` module) when a Core Audio default
/// device change requires re-binding a Stream. Set to `Some(sender)` for
/// the lifetime of an active live-transcription session, `None`
/// otherwise. The broker checks the inbox before deciding whether to
/// route a restart through the live loop (which knows how to reset
/// `SourceVadState`) or to call `AudioManager::restart_*` directly.
pub type RestartIntentSender = tokio::sync::mpsc::UnboundedSender<RestartIntent>;
pub type RestartIntentInbox = Arc<StdMutex<Option<RestartIntentSender>>>;

/// What the broker is asking the live loop to do. Narrow on purpose —
/// the loop doesn't need a "target device id" because it always rebinds
/// to the current OS default (the broker has already debounced and
/// confirmed `is_device_alive`).
#[derive(Debug, Clone, Copy)]
pub enum RestartIntent {
    Mic,
    System,
}

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
    /// When the defensive device-identity drift poll last ran. Throttled to
    /// a few seconds so the `cpal::default_host()` call isn't made on every
    /// ~100–300 ms loop tick.
    last_device_check_at: Option<Instant>,
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
            last_device_check_at: None,
            silero: SileroSource::new(initial_pos),
        }
    }

    /// Resets all buffer-position, VAD, and source-format state after the
    /// underlying ring buffer has been replaced (e.g. on a device-format
    /// change during restart). Positions from the old buffer are not
    /// meaningful against the new one, and the sample rate / channel count
    /// must track the new device's format so extraction and WAV writes stay
    /// correct. Preserves cross-buffer state: chunk_index, accumulated_text.
    fn reset_for_buffer_replacement(&mut self, new_pos: usize, sample_rate: u32, channels: u16) {
        self.is_speaking = false;
        self.speech_start_pos = new_pos;
        self.cursor = new_pos;
        self.speech_start_time = None;
        self.silence_since = None;
        self.session_start_pos = new_pos;
        self.source_sample_rate = sample_rate;
        self.source_channels = channels;
        self.last_seen_write_pos = new_pos;
        self.last_write_pos_advance = Instant::now();
        self.earliest_next_chunk_pos = new_pos;
        self.silero = SileroSource::new(new_pos);
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
/// - Parakeet: meeting-tuned cadence (500 ms silence, 10 s max
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

/// Engine-conditional configuration consolidated into a single value
/// resolved once at session start. The live loop and `transcribe_chunk`
/// consult this instead of re-deriving from `EngineKind` at every site —
/// keeps the path clean of `match engine_kind` sprawl as the upcoming
/// queue / watchdog / repair stages add more engine-aware behavior.
#[derive(Debug, Clone)]
struct EngineProfile {
    /// Underlying engine identity. Kept here so call sites that need the
    /// raw enum (hallucination filtering, IPC requests) don't have to pass
    /// it separately alongside the profile.
    engine_kind: yapstack_common::types::EngineKind,
    /// Display name for telemetry surfaces — `"Whisper"` / `"Parakeet"`.
    engine_name: &'static str,
    /// Per-source VAD timing + thresholds.
    vad_tuning: VadTuning,
    /// Whisper's whisper-rs decoder consumes `initial_prompt` (vocabulary
    /// hints + rolling accumulated_text); Parakeet TDT's decoder ignores
    /// any text prompt. Drives prompt-context build / decay sites so
    /// neither engine has to be matched on directly.
    uses_initial_prompt: bool,
    /// Resolved acceleration label captured from the live
    /// `TranscriptionClient`'s engine_info at session start. Populated
    /// when the FE knows what's running; `None` for older sidecars or
    /// when the spawn-time query failed (`init_transcription_client`
    /// already logs that case).
    accel: Option<String>,
    /// For Parakeet, the variant directory name (e.g.
    /// `"parakeet-tdt-v3-int8"`). Lets the pressure-event log tell
    /// int8 from fp32 sessions without external joins.
    variant: Option<String>,
}

fn profile_for(
    engine: yapstack_common::types::EngineKind,
    config: &LiveTranscriptionConfig,
) -> EngineProfile {
    use yapstack_common::types::EngineKind;
    match engine {
        // Whisper: preserve existing dictation-proven *timing* exactly.
        // Silence window honors the user's `silence_duration_ms` (frontend
        // default 800 ms); 300 ms poll cadence; no pre-roll. Only the
        // detector swaps RMS → Silero — all timing constants stay put.
        EngineKind::Whisper => EngineProfile {
            engine_kind: engine,
            engine_name: "Whisper",
            vad_tuning: VadTuning {
                speech_threshold: SPEECH_THRESHOLD,
                offset_threshold_ratio: SILENCE_THRESHOLD / SPEECH_THRESHOLD,
                silence_duration: Duration::from_millis(config.silence_duration_ms as u64),
                max_chunk_duration: Duration::from_secs_f32(config.max_chunk_seconds),
                poll_interval: Duration::from_millis(POLL_INTERVAL_MS),
                pre_roll: Duration::ZERO,
            },
            uses_initial_prompt: true,
            accel: None,
            variant: None,
        },
        // Parakeet: meeting-tuned. Ignores frontend silence / chunk / poll
        // knobs — these are engine-specific best practice, not user-facing
        // tuning.
        //
        // `silence_duration` was 200 ms originally, which mirrors a
        // dictation-style "fire as soon as the speaker pauses" cadence.
        // For multi-speaker meeting transcription that produces ~3× more
        // dispatches than necessary — every comma-breath triggers a chunk —
        // and the IPC + inference pressure of those extra dispatches is the
        // proximate cause of the stall the cap commit (e6b05ea) was trying
        // to bound. Comparable production stacks use 500 ms (Vad+Whisper
        // dictation), 750 ms (FluidAudio meeting mode), or higher (Muesli
        // aims for 3 s minimum chunks). Bumped to 500 ms — the conservative
        // half of that range — to cut dispatch rate by ~2-3× on fast
        // multi-speaker dialogue while keeping live-caption feel.
        //
        // `max_chunk_duration` stays at 10 s for now: the cap commit
        // documented chunks ≥12 s degrading to RTFx < 1 on our CPU path.
        // FluidAudio's 14.4 s window is feasible because they run on the
        // ANE; we don't have that path until we either (a) get TDT v3
        // working on ORT-CoreML or (b) switch to streaming Nemotron.
        // Stage 6 of the pipeline overhaul plan revisits once we have
        // real per-chunk wall-time data to validate.
        EngineKind::Parakeet => EngineProfile {
            engine_kind: engine,
            engine_name: "Parakeet",
            vad_tuning: VadTuning {
                speech_threshold: SPEECH_THRESHOLD,
                offset_threshold_ratio: SILENCE_THRESHOLD / SPEECH_THRESHOLD,
                silence_duration: Duration::from_millis(500),
                max_chunk_duration: Duration::from_secs(10),
                poll_interval: Duration::from_millis(100),
                pre_roll: Duration::from_millis(250),
            },
            uses_initial_prompt: false,
            accel: None,
            variant: None,
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
    lag_seconds: Option<f32>,
    cap_fired_total: u32,
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
            lag_seconds,
            cap_fired_total,
        },
    );
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

/// Boxed future that yields a `ChunkTaskOutcome` once a spawned chunk
/// transcribe finishes (or its panic-recovery handler synthesizes one).
/// Held by `live_transcription_loop`'s `FuturesUnordered` and drained by the
/// post-loop helpers.
type ChunkTaskFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = ChunkTaskOutcome> + Send>>;

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
    session_offset_base_seconds: f32,
    max_chunk_duration: Duration,
) -> Option<PreparedChunk> {
    let (extraction, new_pos) = {
        let manager = audio_state.lock().await;
        extract_source_audio(&manager, &vad.label, vad.speech_start_pos)
    };

    let Some((mut samples, sample_rate)) = extraction else {
        // Nothing new in the buffer since last read — advance the cursor to
        // the latest write_pos but leave speech_start_pos so we'll pick up
        // the ongoing utterance on the next poll.
        vad.cursor = new_pos;
        return None;
    };

    let extracted_duration = samples.len() as f32 / sample_rate as f32;
    if extracted_duration < MIN_CHUNK_DURATION_SECS {
        // Too short — don't dispatch yet and don't advance speech_start_pos.
        // Next poll re-extracts this region together with whatever arrives.
        vad.cursor = new_pos;
        return None;
    }

    // Cap the dispatched chunk at `max_chunk_duration`. When a prior chunk
    // sits in flight longer than its own audio duration (e.g. Parakeet's
    // RTFx degrades on long inputs), the next dispatch otherwise covers the
    // entire wait window plus 10 s — and the chunk after that grows again,
    // ad infinitum. Latency-first policy: keep the *tail* (most recent
    // audio) and drop the head, so the live transcript catches up to "now"
    // instead of replaying a stale backlog. `speech_start_pos` is still
    // advanced to `new_pos` below, so the dropped head is permanently gone
    // — the alternative (rewind speech_start_pos to mid-extraction) would
    // re-queue the dropped head on the next poll and reintroduce the
    // unbounded queue we're fixing.
    let max_samples = (max_chunk_duration.as_secs_f32() * sample_rate as f32) as usize;
    let dropped_head_samples = samples.len().saturating_sub(max_samples);
    let dropped_head_seconds = dropped_head_samples as f32 / sample_rate as f32;
    if dropped_head_samples > 0 {
        // Structured marker for grep-friendly capture in long sessions.
        // Stage 3 deletes this entire branch — until then, every firing
        // means audio was permanently lost.
        warn!(
            marker = "live_cap_fired",
            source = ?vad.label,
            extracted_secs = extracted_duration,
            max_chunk_secs = max_chunk_duration.as_secs_f32(),
            dropped_secs = dropped_head_seconds,
            "live chunk: extracted audio exceeds max_chunk_duration — dropping head \
             (likely sidecar wall time exceeded chunk duration)"
        );
        samples.drain(..dropped_head_samples);
    }
    let chunk_duration = samples.len() as f32 / sample_rate as f32;

    // Deterministic offset from buffer position delta. On a resumed Session,
    // `session_offset_base_seconds` shifts every live offset past the prior
    // parts' cumulative duration so persisted Segments stay continuous. When
    // we drop the head, the offset shifts forward by the dropped duration so
    // segment timestamps still match the audio we actually sent.
    let samples_since_start = vad.speech_start_pos.saturating_sub(vad.session_start_pos);
    let audio_offset = session_offset_base_seconds
        + samples_since_start as f32 / (vad.source_sample_rate as f32 * vad.source_channels as f32)
        + dropped_head_seconds;

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
        dropped_head_seconds,
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
    /// How many seconds of audio the head-drop cap discarded before
    /// dispatching this chunk. Zero on the normal path. Reported via
    /// `SessionCounters::cap_fired_total` so the UI can surface that audio
    /// was lost. Stage 3 removes the cap and this field with it.
    dropped_head_seconds: f32,
}

/// Run a single chunk's transcription in a background task. Emits the
/// segment event on success; quarantines the audio and emits a warning on
/// failure. Returns the outcome so the main loop can restore
/// `accumulated_text` and notice a dead sidecar.
async fn run_chunk_task(
    prepared: PreparedChunk,
    ctx: TranscriptionContext,
    counters: Arc<StdMutex<SessionCounters>>,
    prompt: Arc<StdMutex<PromptState>>,
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
        &counters,
        &prompt,
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
/// Throttle for the defensive device-identity drift poll. Primary detection
/// is the push-based CoreAudio property listener (Layer 0); this poll only
/// covers the rare case where listener registration fails or the OS drops
/// an event. A short throttle caps the cost of the periodic
/// `cpal::default_host()` lookup.
const DEVICE_IDENTITY_POLL_INTERVAL_SECS: f32 = 3.0;

// --- Pure stream health decision helpers ---

/// Returns `true` if a write-position stall should trigger a stream restart for the
/// given source. On Windows, system audio loopback produces zero samples when nothing
/// is playing — this is normal WASAPI behavior, not a stream failure.
fn should_stall_restart(label: &AudioSourceLabel) -> bool {
    !(cfg!(target_os = "windows") && matches!(label, AudioSourceLabel::System))
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

    // Snapshot symptom-layer state + consume any pending default-device-
    // change notification. Consuming the flag here means a listener event
    // fired between sessions still triggers a rebind on preflight, before
    // first extraction reads stale audio.
    let (
        mic_initial,
        sys_initial,
        mic_err,
        sys_err,
        has_mic_buf,
        has_sys_buf,
        mic_default_changed,
        sys_default_changed,
        mic_drift,
        sys_drift,
    ) = {
        let m = audio_state.lock().await;
        (
            m.mic_write_pos(),
            m.system_write_pos(),
            m.mic_has_stream_error(),
            m.system_has_stream_error(),
            m.mic_buffer().is_some(),
            m.system_buffer().is_some(),
            m.mic_default_changed(),
            m.system_audio_default_changed(),
            m.mic_input_drifted(),
            m.system_audio_output_drifted(),
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
        let device_changed = mic_default_changed || mic_drift.is_some();
        if mic_err || stalled || device_changed {
            warn!(
                "preflight: mic stream needs restart (error={}, stalled={}, device_changed={})",
                mic_err, stalled, device_changed
            );
            match manager.restart_mic() {
                Ok(_) => restarted.push(AudioSourceLabel::Mic),
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
        let device_changed = sys_default_changed || sys_drift.is_some();
        if sys_err || stalled || device_changed {
            warn!(
                "preflight: system stream needs restart (error={}, stalled={}, device_changed={})",
                sys_err, stalled, device_changed
            );
            match manager.restart_system_audio() {
                Ok(_) => restarted.push(AudioSourceLabel::System),
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
                            bound_device_name: None,
                        },
                    );
                }
            }
        }
    }

    drop(manager);

    for label in restarted {
        let name = source_display_name(&label);
        let bound_device_name = {
            let manager = audio_state.lock().await;
            match label {
                AudioSourceLabel::Mic => manager.mic_bound_device().map(|s| s.to_string()),
                AudioSourceLabel::System => {
                    manager.system_audio_bound_device().map(|s| s.to_string())
                }
            }
        };
        let _ = app_handle.emit(
            "stream-health",
            StreamHealthEvent {
                source: label,
                status: "restarted".into(),
                message: format!("{name} stream restarted (preflight)"),
                bound_device_name,
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
    // Pre-roll must not rewind past the last emitted chunk's end, or the
    // next chunk would re-transcribe the tail of the previous one.
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

/// Process backfill audio concurrently with the live VAD loop. Each source's
/// chunk boundaries come from `vad_chunk_historical_audio`, which runs the
/// same Silero-driven state machine the live loop uses; backfill and live
/// segmentation share boundary choices (no quality gap between a session's
/// first N seconds and the rest). Emits segments with offsets in the backfill
/// window, then sets `backfill_done`.
///
/// `counters` is the same `Arc<StdMutex<SessionCounters>>` the live loop
/// holds — backfill chunks update the same totals the live loop and
/// `get_live_transcription_status` read, so the status surface reflects
/// backfill + live work together. Backfill owns its **own** `PromptState`
/// (built locally below) so the prompt-bridge step at end-of-backfill can't
/// race with concurrent live writes — once bridging is done, the bridged
/// text flows through `ctx.bridged_prompt` to the live loop's
/// `seed_prompt_from_backfill` instead.
async fn process_backfill(
    ctx: TranscriptionContext,
    backfill_audio: Vec<(Vec<f32>, u32, AudioSourceLabel)>,
    backfill_done: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    tuning: VadTuning,
    counters: Arc<StdMutex<SessionCounters>>,
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

    // Backfill-local prompt accumulator. Kept separate from the live loop's
    // `PromptState` so the bridge step below can `take` it without racing
    // concurrent live writes.
    let backfill_prompt = Arc::new(StdMutex::new(PromptState {
        shared_prompt: String::new(),
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
                &counters,
                &backfill_prompt,
            )
            .await;
        }
    }

    {
        let s = counters.lock().expect("counters mutex poisoned");
        info!(
            "backfill: completed {} chunks, {:.1}s audio",
            s.total_chunks, s.total_audio_seconds
        );
    }

    // Bridge backfill's accumulated prompt out to the live loop. Since
    // `backfill_prompt` is owned only by this task, taking from it here
    // can't race with live chunks.
    {
        let mut bridged = ctx.bridged_prompt.lock().await;
        let max_prompt = ctx.config.prompt_context_chars.unwrap_or(350) as usize;
        let mut bp = backfill_prompt
            .lock()
            .expect("backfill prompt mutex poisoned");
        if bp.shared_prompt.len() > max_prompt {
            let boundary = bp
                .shared_prompt
                .ceil_char_boundary(bp.shared_prompt.len() - max_prompt);
            *bridged = bp.shared_prompt[boundary..].to_string();
        } else {
            *bridged = std::mem::take(&mut bp.shared_prompt);
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

/// Per-tick stream-health pass over every source. Each source is checked
/// against three layered signals — OS-authoritative listener events first,
/// then symptom-based detection (cpal error flag, write-position stall,
/// device-identity drift) gated by a cooldown — and restarted when any one
/// of them fires. `session_wav_state` is threaded through so a
/// buffer-replacing restart can reset the WAV writer's flush position;
/// otherwise the old position against the fresh buffer stalls writes until
/// the new counter climbs past it.
async fn check_stream_health(
    sources: &mut [SourceVadState],
    audio_state: &AudioManagerState,
    app_handle: &AppHandle,
    mut session_wav_state: Option<&mut SessionWavState>,
    is_mixed: bool,
) -> bool {
    for source in sources.iter_mut() {
        if source.restart_attempts >= STREAM_RESTART_MAX_ATTEMPTS {
            continue;
        }

        let mut reason = evaluate_listener_signal(source, audio_state).await;

        if reason.is_none() {
            let in_cooldown = source.last_restart_at.is_some_and(|t| {
                t.elapsed() < Duration::from_secs_f32(STREAM_RESTART_COOLDOWN_SECS)
            });
            if !in_cooldown {
                reason = evaluate_speculative_signals(source, audio_state).await;
            }
        }

        let Some(reason) = reason else { continue };
        // `as_deref_mut` reborrows the `Option<&mut _>` so each loop
        // iteration gets a fresh mutable view without consuming the slot.
        #[allow(clippy::needless_option_as_deref)]
        let ws_borrow = session_wav_state.as_deref_mut();
        attempt_source_restart(source, audio_state, app_handle, ws_borrow, &reason).await;
    }

    // Mixed mid-capture fail-fast. The user's intent with Mixed is
    // "capture both", not "limp along on the surviving Source", so a
    // terminal restart failure on either side ends the whole Capture.
    // The per-source `restart_abandoned` toast was already emitted from
    // inside `attempt_source_restart`; no extra event needed here.
    is_mixed
        && sources
            .iter()
            .any(|s| s.restart_attempts >= STREAM_RESTART_MAX_ATTEMPTS)
}

/// Layer 0: OS-authoritative rebind signal. Returns `Some(reason)` only when
/// the CoreAudio listener fired AND a re-query after a 200 ms settle confirms
/// the default device is genuinely different from what's bound — macOS can
/// momentarily revert the default during a Bluetooth handshake (cpal#1175),
/// so the settle-and-recheck is what keeps us from rebinding to a still-dead
/// device. Listener signals bypass the speculative-restart cooldown because
/// they're a real OS push, not a guess.
async fn evaluate_listener_signal(
    source: &SourceVadState,
    audio_state: &AudioManagerState,
) -> Option<String> {
    let listener_fired = {
        let manager = audio_state.lock().await;
        let default_changed = match source.label {
            AudioSourceLabel::Mic => manager.mic_default_changed(),
            AudioSourceLabel::System => manager.system_audio_default_changed(),
        };
        // Device-list change is a shared signal consumed once per tick; fold
        // it into the listener-fired decision for the first source that
        // inspects it.
        let devices_changed = manager.device_list_changed();
        default_changed || devices_changed
    };
    if !listener_fired {
        return None;
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    let manager = audio_state.lock().await;
    let bound = match source.label {
        AudioSourceLabel::Mic => manager.mic_bound_device(),
        AudioSourceLabel::System => manager.system_audio_bound_device(),
    }
    .map(str::to_string);
    let current_default = match source.label {
        AudioSourceLabel::Mic => yapstack_audio::manager::live_default_input_name(),
        AudioSourceLabel::System => yapstack_audio::manager::live_default_output_name(),
    };
    let direction = match source.label {
        AudioSourceLabel::Mic => "input",
        AudioSourceLabel::System => "output",
    };
    match (bound.as_deref(), current_default.as_deref()) {
        (Some(b), Some(c)) if b != c => {
            info!(
                "default {} device change confirmed after settle: '{}' → '{}', rebinding",
                direction, b, c
            );
            Some("default device changed".into())
        }
        (Some(b), Some(c)) => {
            // Listener fired but the default unchanged once the OS finished
            // settling — spurious / transient signal. Let the stall watchdog
            // catch a real disconnect.
            debug!(
                "listener fired for {} but default unchanged after settle (still '{}' ≈ '{}') — skipping restart",
                direction, b, c
            );
            None
        }
        _ => {
            // Bound name unavailable (stream never started or no device
            // resolved). Trust the listener and rebind.
            info!(
                "default {} device changed (bound='{:?}', current='{:?}'), rebinding",
                direction, bound, current_default
            );
            Some("default device changed".into())
        }
    }
}

/// Layers 1–3: symptom-based detection, gated by the speculative-restart
/// cooldown in the caller. Layer 1 is the cpal error-callback flag (instant);
/// Layer 2 is the write-position stall watchdog (~2 s); Layer 3 is the
/// defensive device-identity drift poll that catches a missed Layer-0 event.
/// Returns the first reason that fires, or `None` if none do.
async fn evaluate_speculative_signals(
    source: &mut SourceVadState,
    audio_state: &AudioManagerState,
) -> Option<String> {
    let manager = audio_state.lock().await;

    // Layer 1: cpal error callback flag.
    let has_error = match source.label {
        AudioSourceLabel::Mic => manager.mic_has_stream_error(),
        AudioSourceLabel::System => manager.system_has_stream_error(),
    };
    if has_error {
        return Some("stream error callback fired".into());
    }

    // Layer 2: write_pos stall detection. On Windows, system audio loopback
    // produces zero samples when nothing is playing — that's normal WASAPI
    // behavior, so `should_stall_restart` skips that case.
    let current_pos = source_write_pos(&manager, &source.label);
    if current_pos > source.last_seen_write_pos {
        source.last_seen_write_pos = current_pos;
        source.last_write_pos_advance = Instant::now();
    } else if source.last_write_pos_advance.elapsed()
        > Duration::from_secs_f32(STREAM_STALL_THRESHOLD_SECS)
        && should_stall_restart(&source.label)
    {
        return Some("write position stalled".into());
    }

    // Layer 3: defensive device-identity drift poll. Throttled.
    let should_check = source
        .last_device_check_at
        .is_none_or(|t| t.elapsed() > Duration::from_secs_f32(DEVICE_IDENTITY_POLL_INTERVAL_SECS));
    if should_check {
        source.last_device_check_at = Some(Instant::now());
        let drift = match source.label {
            AudioSourceLabel::Mic => manager.mic_input_drifted(),
            AudioSourceLabel::System => manager.system_audio_output_drifted(),
        };
        if let Some(new_name) = drift {
            let bound = match source.label {
                AudioSourceLabel::Mic => manager.mic_bound_device(),
                AudioSourceLabel::System => manager.system_audio_bound_device(),
            }
            .map(str::to_string)
            .unwrap_or_else(|| "<unknown>".into());
            warn!(
                "device identity drift without listener event: '{}' → '{}' (rebinding)",
                bound, new_name
            );
            return Some("device identity drift (listener missed)".into());
        }
    }

    None
}

/// Run a single restart attempt for `source`. Records the attempt timestamp,
/// invokes the engine's restart entry point, and on success handles the
/// buffer-replacement bookkeeping (cursor reset + WAV flush-position reset).
/// On a same-device rebind, treats the attempt as partial — increments the
/// retry counter and clears the cooldown so the next tick can try again
/// immediately instead of waiting 5 s. Emits a `stream-health` event in
/// every outcome.
async fn attempt_source_restart(
    source: &mut SourceVadState,
    audio_state: &AudioManagerState,
    app_handle: &AppHandle,
    mut session_wav_state: Option<&mut SessionWavState>,
    reason: &str,
) {
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
        Ok(report) => {
            info!(
                "stream health: {} restarted successfully (outcome: {:?}, same_device: {}, new_id: {:?})",
                source_name, report.outcome, report.same_device, report.new_device_id
            );

            // Same-device rebind likely means macOS was still settling after
            // a Bluetooth handshake (cpal#1175). Treat it like a partial
            // failure: keep counting attempts and clear the cooldown so the
            // next tick can retry immediately. Do NOT reset
            // last_write_pos_advance — the stream hasn't actually recovered,
            // and Layer 2 should fire on the next tick if write_pos isn't
            // advancing.
            if report.same_device {
                warn!(
                    "{} restart rebound to the same device ({:?}) — macOS likely still settling; \
                     retrying on next poll (attempt {}/{})",
                    source_name,
                    report.new_device_id,
                    source.restart_attempts + 1,
                    STREAM_RESTART_MAX_ATTEMPTS
                );
                source.restart_attempts += 1;
                source.last_restart_at = None;
            } else {
                source.restart_attempts = 0;
                source.last_write_pos_advance = Instant::now();
            }

            if report.outcome == yapstack_audio::manager::RestartOutcome::BufferReplaced {
                #[allow(clippy::needless_option_as_deref)]
                let ws_borrow = session_wav_state.as_deref_mut();
                handle_buffer_replacement(source, audio_state, ws_borrow, source_name).await;
            }

            let _ = app_handle.emit(
                "stream-health",
                StreamHealthEvent {
                    source: source.label,
                    status: "restarted".into(),
                    message: format!("{} stream restarted ({})", source_name, reason),
                    bound_device_name: report.bound_device_name.clone(),
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
                    bound_device_name: None,
                },
            );
        }
    }
}

/// Buffer-replaced restart bookkeeping. Reads the fresh buffer's metadata
/// (sample rate, channels, write_pos) and resets every cursor on the source
/// to point into the new buffer. Also resets just this source's
/// `flush_positions` on the WAV writer — leaving the old position would stall
/// WAV writes until the new buffer's counter climbs past it. Sample-rate
/// mismatch is logged; the per-tick write path resamples on the fly.
async fn handle_buffer_replacement(
    source: &mut SourceVadState,
    audio_state: &AudioManagerState,
    session_wav_state: Option<&mut SessionWavState>,
    source_name: &str,
) {
    let (new_pos, sr, ch) = {
        let manager = audio_state.lock().await;
        let info = match source.label {
            AudioSourceLabel::Mic => manager.mic_buffer_info(),
            AudioSourceLabel::System => manager.system_buffer_info(),
        };
        let pos = source_write_pos(&manager, &source.label);
        info.map(|i| (pos, i.sample_rate, i.channels)).unwrap_or((
            pos,
            source.source_sample_rate,
            source.source_channels,
        ))
    };
    source.reset_for_buffer_replacement(new_pos, sr, ch);
    warn!(
        "{} buffer replaced on restart ({}Hz/{}ch) — source state reset",
        source_name, sr, ch
    );

    if let Some(ws) = session_wav_state {
        match source.label {
            AudioSourceLabel::Mic => ws.flush_positions.mic_pos = new_pos,
            AudioSourceLabel::System => ws.flush_positions.system_pos = new_pos,
        }
        if sr != ws.wav_sample_rate {
            info!(
                "WAV sample-rate mismatch on {} rebind: writer={}Hz, new buffer={}Hz — extracted samples will be resampled on write",
                source_name, ws.wav_sample_rate, sr
            );
        }
    }
}

async fn live_transcription_loop(
    audio_state: AudioManagerState,
    ctx: TranscriptionContext,
    mut stop_rx: oneshot::Receiver<()>,
    mut restart_intent_rx: tokio::sync::mpsc::UnboundedReceiver<RestartIntent>,
    mut session_wav_state: Option<SessionWavState>,
    counters: Arc<StdMutex<SessionCounters>>,
) {
    // Live-loop-private prompt state. Backfill owns its own (see
    // `process_backfill`); they bridge via `ctx.bridged_prompt`.
    let prompt = Arc::new(StdMutex::new(PromptState {
        shared_prompt: String::new(),
        last_transcription_at: None,
    }));
    let source = ctx.config.source.clone().into();
    let check_mic = matches!(source, CaptureSource::MicOnly | CaptureSource::Mixed);
    let check_system = matches!(source, CaptureSource::SystemOnly | CaptureSource::Mixed);

    // VAD tuning resolved at session start (see start_live_transcription)
    // and shared via `ctx.engine_profile`. Re-bind locally so the rest of
    // the loop body can keep its existing `tuning.*` field accesses.
    let tuning = ctx.engine_profile.vad_tuning;

    let (mut sources, backfill_audio) = build_initial_sources_and_backfill(
        &audio_state,
        &ctx.config,
        check_mic,
        check_system,
        source,
    )
    .await;

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
        let backfill_counters = counters.clone();
        let handle = tokio::spawn(process_backfill(
            backfill_ctx,
            backfill_audio,
            backfill_done_clone,
            backfill_cancel_clone,
            tuning,
            backfill_counters,
        ));
        let abort_handle = handle.abort_handle();
        Some((handle, abort_handle))
    } else {
        backfill_done.store(true, Ordering::Release);
        None
    };

    let mut wav_flush_none_count: u32 = 0;
    let mut prompt_seeded_from_backfill = false;

    emit_status(
        &ctx.app_handle,
        LiveTranscriptionPhase::Running,
        0,
        0.0,
        None,
        0,
    );

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
            emit_status(
                &ctx.app_handle,
                LiveTranscriptionPhase::Error,
                0,
                0.0,
                None,
                0,
            );
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
            // Device broker is asking us to fail over a Source. We honor
            // it by calling the existing `attempt_source_restart` path
            // (which resets `SourceVadState` cursor + Silero state and
            // updates the session-WAV flush position). The broker has
            // already debounced and `is_device_alive`-gated, so we don't
            // re-check those here. The legacy listener-flag poll is
            // still alive (Phase 8 removes it); whichever wins, the
            // existing throttle in `attempt_source_restart` collapses
            // duplicates.
            Some(intent) = restart_intent_rx.recv() => {
                let target_label = match intent {
                    RestartIntent::Mic => AudioSourceLabel::Mic,
                    RestartIntent::System => AudioSourceLabel::System,
                };
                if let Some(source) = sources.iter_mut().find(|s| s.label == target_label) {
                    let ws_borrow = session_wav_state.as_mut();
                    attempt_source_restart(
                        source,
                        &audio_state,
                        &ctx.app_handle,
                        ws_borrow,
                        "device-change",
                    )
                    .await;
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

        if let Some((samples, sr, new_pos)) = wav_flush_data {
            wav_flush_none_count = 0;
            if let Some(ref mut ws) = session_wav_state {
                if matches!(
                    write_session_wav_samples(ws, &samples, sr, new_pos),
                    WavWriteOutcome::ResampleFailed
                ) {
                    continue;
                }
            }
        } else if session_wav_state.is_some() {
            handle_empty_wav_flush(
                &mut wav_flush_none_count,
                session_wav_state.as_ref(),
                &audio_state,
                &ctx,
                &backfill_done,
            )
            .await;
        }

        // Stream health watchdog: check for cpal error flags and write_pos stalls.
        // Triggers auto-restart if a stream has died silently.
        // restart_mic() tries the previously stored device first, then falls back to
        // the provided name (None = system default).
        let mixed_fail_fast = check_stream_health(
            &mut sources,
            &audio_state,
            &ctx.app_handle,
            session_wav_state.as_mut(),
            matches!(source, CaptureSource::Mixed),
        )
        .await;

        // Mixed mid-capture fail-fast: stop both Sources and exit. The
        // per-Source `restart_abandoned` toast was already emitted; the
        // session naturally winds down through the standard cleanup
        // path (drain in-flight chunks, finalize WAV, etc.).
        if mixed_fail_fast {
            warn!(
                "Mixed capture: terminal restart failure on a Source — ending session"
            );
            let mut manager = audio_state.lock().await;
            let _ = manager.stop_all();
            drop(manager);
            break;
        }

        if !prompt_seeded_from_backfill
            && seed_prompt_from_backfill(&ctx, &prompt, &mut sources).await
        {
            prompt_seeded_from_backfill = true;
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

                let mut best_action = VadAction::None;
                if probs.is_empty() {
                    // Sticky fallback: no new frames in this batch. Use
                    // last_probability so the state machine keeps running
                    // against the most recent signal rather than freezing.
                    let action = poll_vad(source, source.silero.last_probability, &tuning);
                    if matches!(action, VadAction::Chunk | VadAction::ForceChunk) {
                        best_action = action;
                    }
                } else {
                    for &prob in probs {
                        let action = poll_vad(source, Some(prob), &tuning);
                        match action {
                            VadAction::ForceChunk => best_action = VadAction::ForceChunk,
                            VadAction::Chunk if !matches!(best_action, VadAction::ForceChunk) => {
                                best_action = VadAction::Chunk;
                            }
                            _ => {}
                        }
                    }
                }

                if should_stop && source.is_speaking {
                    best_action = VadAction::Chunk;
                }
                best_action
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
                    let prepared = prepare_chunk_dispatch(
                        &mut sources[i],
                        &audio_state,
                        is_force,
                        false,
                        ctx.session_offset_base_seconds,
                        tuning.max_chunk_duration,
                    )
                    .await;
                    if let Some(prepared) = prepared {
                        if prepared.dropped_head_seconds > 0.0 {
                            let mut s = counters.lock().expect("counters mutex poisoned");
                            s.cap_fired_total = s.cap_fired_total.saturating_add(1);
                        }
                        let task_ctx = ctx.clone();
                        let task_counters = counters.clone();
                        let task_prompt = prompt.clone();
                        let source_label = prepared.source_label;
                        let fallback_text = prepared.accumulated_text.clone();
                        let handle = tokio::spawn(async move {
                            run_chunk_task(prepared, task_ctx, task_counters, task_prompt).await
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
            emit_fatal_sidecar_error(&ctx, &counters);
            exited_fatal = true;
            break;
        }

        run_prompt_decay(&ctx, &prompt, &mut sources);

        if should_stop {
            break;
        }
    }

    drain_in_flight_chunks(&mut chunk_tasks, &mut chunk_aborts, &mut sources).await;
    dispatch_final_pending_chunks(
        &mut sources,
        &audio_state,
        &ctx,
        &counters,
        &prompt,
        tuning.max_chunk_duration,
    )
    .await;

    if let Some(ws) = session_wav_state {
        finalize_session_wav(ws, &audio_state, &ctx).await;
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

    let (final_chunks, final_audio_seconds, final_cap_fired, final_lag, final_total_wall_ms) = {
        let s = counters.lock().expect("counters mutex poisoned");
        let lag = s.latest_completed_audio_offset_seconds.map(|chunk_end| {
            let session_time_now =
                ctx.session_offset_base_seconds + ctx.session_start_instant.elapsed().as_secs_f32();
            (session_time_now - chunk_end).max(0.0)
        });
        (
            s.total_chunks,
            s.total_audio_seconds,
            s.cap_fired_total,
            lag,
            s.total_wall_ms,
        )
    };

    // Only emit Stopped if we didn't already emit Error (avoids duplicate finalization)
    if !exited_fatal {
        emit_status(
            &ctx.app_handle,
            LiveTranscriptionPhase::Stopped,
            final_chunks,
            final_audio_seconds,
            final_lag,
            final_cap_fired,
        );
    }

    // Session pressure summary — one structured line per session so log
    // captures from a stalled run carry the aggregate signal even when the
    // per-chunk lines are too verbose to scan.
    let mean_rtfx = if final_total_wall_ms > 0 && final_audio_seconds > 0.0 {
        Some(final_audio_seconds / (final_total_wall_ms as f32 / 1000.0))
    } else {
        None
    };
    info!(
        marker = "live_session_summary",
        engine = ctx.engine_profile.engine_name,
        session_id = ?ctx.config.session_id,
        chunks = final_chunks,
        audio_secs = final_audio_seconds,
        wall_ms = final_total_wall_ms,
        mean_rtfx = ?mean_rtfx,
        cap_fired_total = final_cap_fired,
        final_lag_secs = ?final_lag,
        exited_fatal = exited_fatal,
        "live transcription session ended"
    );
}

/// Build the per-source VAD state for the live loop and, if `backfill_seconds`
/// is non-zero, extract the matching pre-session audio and reset each source's
/// cursors / Silero stream forward to the current write position. Reading
/// backfill before the live loop starts guarantees a single coherent snapshot
/// — the loop then begins polling at the post-backfill cursor. Returns the
/// initialized sources alongside the backfill audio queued for concurrent
/// processing.
async fn build_initial_sources_and_backfill(
    audio_state: &AudioManagerState,
    config: &LiveTranscriptionConfig,
    check_mic: bool,
    check_system: bool,
    source: CaptureSource,
) -> (Vec<SourceVadState>, Vec<(Vec<f32>, u32, AudioSourceLabel)>) {
    let manager = audio_state.lock().await;
    let positions = manager.buffer_positions();

    let has_backfill = config.backfill_seconds > 0.0;
    let mut sources: Vec<SourceVadState> = Vec::with_capacity(2);
    let mut backfill: Vec<(Vec<f32>, u32, AudioSourceLabel)> = Vec::new();

    if check_mic {
        let (initial_pos, session_start, sr, ch) = if let Some(buf) = manager.mic_buffer() {
            let sr = buf.sample_rate();
            let ch = buf.channels();
            if has_backfill {
                let raw = (config.backfill_seconds * sr as f32 * ch as f32) as usize;
                // Round down to frame boundary for correct deinterleaving.
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
                let raw = (config.backfill_seconds * sr as f32 * ch as f32) as usize;
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

    if has_backfill {
        info!(
            "live transcription: extracted backfill audio ({:.1}s) for concurrent processing",
            config.backfill_seconds
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
            // Reset the full per-source live VAD state to `current` so the
            // live loop starts from the post-backfill write position. Without
            // this, Silero would replay every backfill sample through the
            // *live* detector on the first few polls, duplicating or delaying
            // speech that backfill already emitted. We also reset the
            // recurrent stream state (LSTM memory was initialized at session
            // start against pre-backfill audio context) and clear the sticky
            // last_probability.
            s.cursor = current;
            s.speech_start_pos = current;
            s.earliest_next_chunk_pos = current;
            s.silero.read_pos = current;
            s.silero.reset();
        }
    }

    (sources, backfill)
}

/// Emit a final `Error`-phase status event after the sidecar has been
/// declared unrecoverable. Reads the running totals out of the shared
/// accumulator so the UI sees the same chunk/audio counts it would on a
/// normal stop.
fn emit_fatal_sidecar_error(ctx: &TranscriptionContext, counters: &Arc<StdMutex<SessionCounters>>) {
    error!("live transcription: sidecar died and could not be restarted — stopping");
    let (chunks, audio_secs, cap_fired_total, lag_seconds) = {
        let s = counters.lock().expect("counters mutex poisoned");
        let lag = s.latest_completed_audio_offset_seconds.map(|chunk_end| {
            let session_time_now =
                ctx.session_offset_base_seconds + ctx.session_start_instant.elapsed().as_secs_f32();
            (session_time_now - chunk_end).max(0.0)
        });
        (
            s.total_chunks,
            s.total_audio_seconds,
            s.cap_fired_total,
            lag,
        )
    };
    let _ = ctx.app_handle.emit(
        "live-transcription-status",
        LiveTranscriptionStatus {
            phase: LiveTranscriptionPhase::Error,
            chunks_processed: chunks,
            total_audio_seconds: audio_secs,
            error_message: Some(
                "Transcription engine stopped unexpectedly and could not be restarted".to_string(),
            ),
            session_id: ctx.config.session_id.clone(),
            effective_start_epoch_ms: None,
            lag_seconds,
            cap_fired_total,
        },
    );
}

/// Per-tick prompt decay check: clear `shared_prompt` and every source's
/// `accumulated_text` once `prompt_decay_silence_seconds` have elapsed since
/// the last successful transcription. Keeps stale context from seeding
/// hallucinations after a long pause.
fn run_prompt_decay(
    ctx: &TranscriptionContext,
    prompt: &Arc<StdMutex<PromptState>>,
    sources: &mut [SourceVadState],
) {
    let prompt_decay_secs = ctx.config.prompt_decay_silence_seconds.unwrap_or(5.0);
    // Hold the lock across read/check/clear/timestamp so the decision is
    // atomic w.r.t. concurrent chunk tasks. Releasing between steps would
    // let a freshly-completed chunk update `shared_prompt` /
    // `last_transcription_at` mid-decay, either clearing fresh prompt text
    // or clobbering a fresh timestamp with `None`.
    let elapsed = {
        let mut p = prompt.lock().expect("prompt mutex poisoned");
        let last_at = p.last_transcription_at;
        let decayed = check_prompt_decay(sources, &mut p.shared_prompt, prompt_decay_secs, last_at);
        if decayed {
            p.last_transcription_at = None;
            Some(last_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0))
        } else {
            None
        }
    };
    if let Some(secs) = elapsed {
        info!(
            "prompt decay: cleared shared_prompt ({:.1}s since last transcription)",
            secs
        );
    }
}

/// Drain still-running chunk tasks at session stop so their segments dispatch
/// before finalization. The frontend's stop finalizer waits on
/// `segmentQueueTail`; that only helps once each task has actually emitted.
/// Phase 1 grants 10 s of graceful drain (long enough for any in-flight
/// transcribe to complete) and restores per-source state from each completed
/// outcome so a source whose final-chunk dispatch was skipped earlier becomes
/// eligible to issue it. Phase 2 aborts on timeout — needed both to keep stop
/// bounded and to release each task's `Arc<TranscriptionClient>` clone so the
/// post-loop `try_unwrap` succeeds.
async fn drain_in_flight_chunks(
    chunk_tasks: &mut futures_util::stream::FuturesUnordered<ChunkTaskFuture>,
    chunk_aborts: &mut Vec<tokio::task::AbortHandle>,
    sources: &mut [SourceVadState],
) {
    if chunk_tasks.is_empty() {
        return;
    }
    debug!(
        "live transcription stop: draining {} in-flight chunk tasks",
        chunk_tasks.len()
    );
    use futures_util::stream::StreamExt;

    let mut drained_outcomes: Vec<ChunkTaskOutcome> = Vec::new();
    let graceful_timed_out = {
        let drain = async {
            while let Some(outcome) = chunk_tasks.next().await {
                if outcome.sidecar_dead {
                    warn!("drained chunk task reported sidecar dead");
                }
                drained_outcomes.push(outcome);
            }
        };
        tokio::time::timeout(Duration::from_secs(10), drain)
            .await
            .is_err()
    };

    for outcome in drained_outcomes {
        for source in sources.iter_mut() {
            if source.label == outcome.source_label {
                source.has_in_flight_task = false;
                source.accumulated_text = outcome.accumulated_text;
                break;
            }
        }
    }

    if graceful_timed_out {
        warn!(
            "live transcription stop: chunk task drain exceeded 10s; \
             aborting remaining tasks to reclaim shared state"
        );
        for ah in chunk_aborts.drain(..) {
            ah.abort();
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while chunk_tasks.next().await.is_some() {}
        })
        .await;
    }
}

/// Final pending-speech dispatch at session stop. Any source whose stop-time
/// `VadAction::Chunk` was skipped because a previous task was still running
/// now has un-transcribed audio between its frozen `speech_start_pos` and the
/// current write position. Run one synchronous chunk per such source so the
/// final segment lands before finalization.
async fn dispatch_final_pending_chunks(
    sources: &mut [SourceVadState],
    audio_state: &AudioManagerState,
    ctx: &TranscriptionContext,
    counters: &Arc<StdMutex<SessionCounters>>,
    prompt: &Arc<StdMutex<PromptState>>,
    max_chunk_duration: Duration,
) {
    for source in sources.iter_mut() {
        if source.has_in_flight_task {
            // Reached only on the Phase 2 abort path — we aborted before an
            // outcome landed. Skipping is safe: that source's state is already
            // considered lost.
            continue;
        }
        let has_pending = {
            let manager = audio_state.lock().await;
            let write_pos = source_write_pos(&manager, &source.label);
            write_pos > source.speech_start_pos
        };
        if !has_pending {
            continue;
        }
        debug!(
            "live transcription stop: dispatching final pending chunk for {:?} \
             (speech_start_pos={}, blocked by prior in-flight task)",
            source.label, source.speech_start_pos
        );
        let prepared = prepare_chunk_dispatch(
            source,
            audio_state,
            true,
            false,
            ctx.session_offset_base_seconds,
            max_chunk_duration,
        )
        .await;
        if let Some(prepared) = prepared {
            if prepared.dropped_head_seconds > 0.0 {
                let mut s = counters.lock().expect("counters mutex poisoned");
                s.cap_fired_total = s.cap_fired_total.saturating_add(1);
            }
            let task_ctx = ctx.clone();
            let task_counters = counters.clone();
            let task_prompt = prompt.clone();
            let source_label = prepared.source_label;
            let final_task = run_chunk_task(prepared, task_ctx, task_counters, task_prompt);
            if tokio::time::timeout(Duration::from_secs(10), final_task)
                .await
                .is_err()
            {
                warn!(
                    "live transcription stop: final pending chunk for {:?} exceeded 10s — \
                     segment may be lost",
                    source_label
                );
            }
        }
    }
}

/// Outcome of a single session-WAV write attempt within the live loop. The
/// only failure the loop cares about is a resample error: when that happens,
/// the loop skips the rest of the tick so it doesn't act on a half-flushed
/// state. Every other path (no `SessionWavState`, write success, write error)
/// is "carry on" from the loop's perspective.
enum WavWriteOutcome {
    Wrote,
    ResampleFailed,
}

/// Append the just-extracted ring-buffer samples to the streaming session WAV.
/// Resamples on the fly when the live extraction sample rate diverges from the
/// header rate (mid-session device rebind), and bumps the periodic-diagnostic
/// counter on success. `flush_positions` is always advanced — retrying a
/// partial write would duplicate samples on the next tick.
fn write_session_wav_samples(
    ws: &mut SessionWavState,
    samples: &[f32],
    sr: u32,
    new_pos: BufferPositions,
) -> WavWriteOutcome {
    let to_write: std::borrow::Cow<[f32]> = if sr == ws.wav_sample_rate {
        std::borrow::Cow::Borrowed(samples)
    } else {
        match yapstack_common::audio::resample(samples, sr, ws.wav_sample_rate) {
            Ok(cow) => std::borrow::Cow::Owned(cow.into_owned()),
            Err(e) => {
                error!(
                    "WAV resample {}Hz → {}Hz failed, dropping {} samples: {}",
                    sr,
                    ws.wav_sample_rate,
                    samples.len(),
                    e
                );
                // Advance positions regardless — retrying would re-extract the
                // same samples next tick.
                ws.flush_positions = new_pos;
                return WavWriteOutcome::ResampleFailed;
            }
        }
    };
    if let Err(e) = ws.writer.write_samples(&to_write) {
        error!(
            "session WAV write error ({} samples may be lost): {}",
            to_write.len(),
            e
        );
    }
    ws.flush_positions = new_pos;
    ws.flush_count += 1;
    if ws.flush_count.is_multiple_of(WAV_FLUSH_DIAGNOSTIC_INTERVAL) {
        debug!(
            "session WAV progress: flushes={}, samples_written={}, duration={:.1}s",
            ws.flush_count,
            ws.writer.samples_written(),
            ws.writer.duration_seconds()
        );
    }
    WavWriteOutcome::Wrote
}

/// Advance the consecutive-empty-flush counter and emit the layered
/// diagnostics: a one-shot user-facing event the first time the count crosses
/// `WAV_FLUSH_ERROR_THRESHOLD` (deferred until backfill completes so the
/// backfill-driven empty window doesn't false-positive), and a periodic warn
/// log every `WAV_FLUSH_WARNING_INTERVAL` ticks thereafter.
async fn handle_empty_wav_flush(
    wav_flush_none_count: &mut u32,
    session_wav_state: Option<&SessionWavState>,
    audio_state: &AudioManagerState,
    ctx: &TranscriptionContext,
    backfill_done: &Arc<AtomicBool>,
) {
    *wav_flush_none_count += 1;
    debug!(
        "session WAV flush: no data (consecutive: {})",
        *wav_flush_none_count
    );
    if *wav_flush_none_count == WAV_FLUSH_ERROR_THRESHOLD {
        if backfill_done.load(Ordering::Acquire) {
            if let Some(ws) = session_wav_state {
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
            *wav_flush_none_count = 0;
        }
    }
    if wav_flush_none_count.is_multiple_of(WAV_FLUSH_WARNING_INTERVAL) {
        let silence_secs = *wav_flush_none_count as f32 * POLL_INTERVAL_MS as f32 / 1000.0;
        warn!(
            "session WAV flush: {} consecutive empty extractions ({:.1}s) — possible sample rate mismatch or no audio data",
            *wav_flush_none_count, silence_secs
        );
    }
}

/// Seed the live shared prompt and per-source `accumulated_text` from the
/// backfill task's bridged prompt the first time it's non-empty. Whisper uses
/// `accumulated_text` as its initial prompt, so seeding both keeps the live
/// loop's first chunk aware of what backfill already transcribed. Returns
/// `true` once the seed lands so the caller can flip the one-shot guard.
async fn seed_prompt_from_backfill(
    ctx: &TranscriptionContext,
    prompt: &Arc<StdMutex<PromptState>>,
    sources: &mut [SourceVadState],
) -> bool {
    let bridged = ctx.bridged_prompt.lock().await;
    if bridged.is_empty() {
        return false;
    }
    {
        let mut p = prompt.lock().expect("prompt mutex poisoned");
        if p.shared_prompt.is_empty() {
            p.shared_prompt = bridged.clone();
        }
        p.last_transcription_at = Some(Instant::now());
    }
    for source in sources.iter_mut() {
        if source.accumulated_text.is_empty() {
            source.accumulated_text = bridged.clone();
        }
    }
    debug!(
        "live loop: seeded prompt from backfill ({} chars)",
        bridged.len()
    );
    true
}

/// Drain the final tail of session audio, finalize the session-WAV writer in
/// the user's chosen export format, and persist the resulting part row to the
/// DB before emitting `session-part-ready`. Called once at end-of-loop after
/// in-flight chunks have drained. Empty recordings get the file deleted and a
/// `session-wav-error` event instead.
async fn finalize_session_wav(
    mut ws: SessionWavState,
    audio_state: &AudioManagerState,
    ctx: &TranscriptionContext,
) {
    let final_flush = {
        let manager = audio_state.lock().await;
        manager.extract_since(&ws.flush_positions, ws.source, ws.mix_config.as_ref())
    };
    if let Some((samples, sr, _new_pos)) = final_flush {
        let to_write: std::borrow::Cow<[f32]> = if sr == ws.wav_sample_rate {
            std::borrow::Cow::Borrowed(&samples)
        } else {
            match yapstack_common::audio::resample(&samples, sr, ws.wav_sample_rate) {
                Ok(cow) => std::borrow::Cow::Owned(cow.into_owned()),
                Err(e) => {
                    error!(
                        "session WAV final flush resample {}Hz → {}Hz failed, dropping {} samples: {}",
                        sr, ws.wav_sample_rate, samples.len(), e
                    );
                    std::borrow::Cow::Owned(Vec::new())
                }
            }
        };
        if !to_write.is_empty() {
            if let Err(e) = ws.writer.write_samples(&to_write) {
                error!("session WAV final flush write failed: {}", e);
            }
        }
    }

    if ws.writer.samples_written() == 0 {
        warn!(
            "session WAV had 0 samples written — deleting empty file for session {}",
            ws.session_id
        );
        let wav_path = ws.writer.path().to_path_buf();
        // Finalize WAV only (no MP3 conversion) to release the file handle, then delete.
        let _ = ws.writer.finalize_wav_only();
        let _ = std::fs::remove_file(&wav_path);
        let _ = ctx.app_handle.emit(
            "session-wav-error",
            SessionWavErrorEvent {
                session_id: ws.session_id,
                message: "No audio was recorded — WAV file not saved".to_string(),
            },
        );
        return;
    }

    let format = ctx
        .config
        .audio_export_format
        .unwrap_or(AudioExportFormatDto::Mp3);
    let result = if format.is_mp3() {
        ws.writer
            .finalize_as_mp3(ctx.config.mp3_bitrate.unwrap_or(64))
    } else {
        ws.writer.finalize_wav_only()
    };
    let (path, duration) = match result {
        Ok(out) => out,
        Err(e) => {
            error!("session WAV finalize failed: {}", e);
            return;
        }
    };

    info!(
        "session part {} finalized: {} ({:.1}s)",
        ws.part_index,
        path.display(),
        duration
    );
    let file_path_str = path.to_string_lossy().to_string();

    // Always register the parent dir with TrustedAudioDirs so the
    // audio-stream:// handler can serve the file regardless of who owns
    // the audio (sessions vs dictations).
    if let Some(parent) = path.parent() {
        crate::register_trusted_audio_dir(&ctx.app_handle, parent);
    }

    // Insert the parts row from Rust *before* emitting so the DB stays the
    // durable source of truth even if the FE event listener is unavailable
    // (crash, force-quit, window closed). The FE handler then just
    // refreshes from DB. Skipped when `persist_audio_part` is false —
    // dictation owns its own audio path on `dictation_history` and the
    // synthetic `session_id` has no row in `sessions`, so inserting would
    // either FK-fail or (with FK enforcement off) leave orphans that
    // `clearAllSessions` could then sweep.
    if ws.persist_audio_part {
        if let Some(db_path_state) = ctx.app_handle.try_state::<crate::DbPath>() {
            let row = crate::db::AudioPartRow {
                session_id: ws.session_id.clone(),
                part_index: ws.part_index,
                file_path: file_path_str.clone(),
                format: format.as_str(),
                duration_seconds: duration,
                sample_rate: ws.wav_sample_rate,
            };
            if let Err(e) = crate::db::insert_audio_part_row(db_path_state.as_path(), &row) {
                error!("insert_audio_part_row failed (will rely on FE refresh): {e}");
            }
        }
    }

    let _ = ctx.app_handle.emit(
        "session-part-ready",
        SessionPartReadyEvent {
            session_id: ws.session_id,
            part_index: ws.part_index,
            file_path: file_path_str,
            format,
            duration_seconds: duration,
            sample_rate: ws.wav_sample_rate,
        },
    );
}

/// Build the `initial_prompt` we'll send to the engine for one chunk:
/// `"<vocabulary_hints>. <rolling_context>"`. Vocabulary hints (folder/tag
/// names) prime Whisper for proper nouns and are read fresh per chunk so
/// mid-recording updates take effect; the hints are snapshotted into an owned
/// string inside a tight scope so the mutex is dropped before the long
/// `transcribe_with` await blocks any concurrent `update_vocabulary_hints`
/// call. Returns `None` only when both vocab and accumulated context are
/// empty.
async fn build_effective_prompt(
    ctx: &TranscriptionContext,
    accumulated_text: &str,
) -> Option<String> {
    let max_prompt = ctx.config.prompt_context_chars.unwrap_or(350) as usize;
    let vocab_truncated: String = {
        let vocab_guard = ctx.vocabulary_hints.lock().await;
        let vocab: &str = vocab_guard.as_str();
        // Round down to a char boundary — folder/tag names can contain
        // multibyte codepoints, and a raw byte slice at the 80-byte cap could
        // land mid-codepoint and panic the live-transcription task.
        let vocab_budget = vocab.floor_char_boundary(vocab.len().min(80));
        vocab[..vocab_budget].to_string()
    };
    let vocab_budget = vocab_truncated.len();
    let context_budget = max_prompt.saturating_sub(if vocab_budget > 0 {
        vocab_budget + 2
    } else {
        0
    });

    if accumulated_text.is_empty() && vocab_budget == 0 {
        return None;
    }

    let context_part = if accumulated_text.is_empty() {
        ""
    } else if accumulated_text.len() > context_budget {
        let boundary = accumulated_text.ceil_char_boundary(accumulated_text.len() - context_budget);
        &accumulated_text[boundary..]
    } else {
        accumulated_text
    };

    if vocab_budget > 0 && !context_part.is_empty() {
        Some(format!("{}. {}", &vocab_truncated, context_part))
    } else if vocab_budget > 0 {
        Some(vocab_truncated)
    } else {
        Some(context_part.to_string())
    }
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
    prompt: &Arc<StdMutex<PromptState>>,
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

    let effective_prompt = build_effective_prompt(ctx, accumulated_text).await;

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
                    lag_seconds: None,
                    cap_fired_total: 0,
                },
            );
            return TranscribeOutcome::SidecarDead;
        }
    };
    // initial_prompt is Whisper-only — Parakeet's TDT decoder has no text-prompt
    // input, so passing it would just be ignored. Drop it explicitly so the IPC
    // payload is honest about what was sent. Engine identity comes from the
    // session-resolved profile so we don't have to re-read `client.engine()`
    // and so future engines can opt in via a single boolean.
    let profile = ctx.engine_profile.as_ref();
    let engine_kind = profile.engine_kind;
    let prompt_for_engine = if profile.uses_initial_prompt {
        effective_prompt.as_deref()
    } else {
        None
    };
    let wall_start = Instant::now();
    let transcription_result = client
        .transcribe_with(
            &wav_path,
            ctx.config.language.as_deref(),
            prompt_for_engine,
            ctx.config.diarization,
        )
        .await;
    let wall_ms = wall_start.elapsed().as_millis() as u64;

    // Emit pressure telemetry regardless of outcome — a chunk that took 8s
    // to fail is just as much "the pipeline is unhealthy" as one that took 8s
    // to succeed. RTFx is only meaningful on success and when wall_ms > 0.
    let rtfx = match (&transcription_result, wall_ms) {
        (Ok(_), w) if w > 0 => Some(chunk_duration / (w as f32 / 1000.0)),
        _ => None,
    };
    let lag_seconds = if transcription_result.is_ok() {
        let session_time_now =
            ctx.session_offset_base_seconds + ctx.session_start_instant.elapsed().as_secs_f32();
        let chunk_end_session_time = input.audio_offset_seconds + chunk_duration;
        Some((session_time_now - chunk_end_session_time).max(0.0))
    } else {
        None
    };
    let _ = ctx.app_handle.emit(
        "live-transcription-pressure",
        LiveTranscriptionPressureEvent {
            source: input.source_label,
            chunk_index: *chunk_index,
            chunk_audio_seconds: chunk_duration,
            wall_ms,
            rtfx,
            engine: profile.engine_name.to_string(),
            is_backfill: input.is_backfill,
            lag_seconds,
            accel: profile.accel.clone(),
            variant: profile.variant.clone(),
        },
    );
    // Structured `info!` mirror of the pressure event so the data lands in
    // the in-app log buffer even when no JS listener is mounted. Every
    // field is grep-friendly and survives across days of usage if log
    // retention does. The "live_pressure" marker makes it easy to filter:
    //   grep 'live_pressure' app.log | jq -s ...
    info!(
        marker = "live_pressure",
        source = ?input.source_label,
        chunk_index = *chunk_index,
        chunk_audio_secs = chunk_duration,
        wall_ms = wall_ms,
        rtfx = ?rtfx,
        lag_seconds = ?lag_seconds,
        engine = profile.engine_name,
        accel = profile.accel.as_deref(),
        variant = profile.variant.as_deref(),
        is_backfill = input.is_backfill,
        ok = transcription_result.is_ok(),
        "transcribe chunk pressure"
    );

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
                let mut p = prompt.lock().expect("prompt mutex poisoned");
                if !p.shared_prompt.is_empty() {
                    p.shared_prompt.push(' ');
                }
                p.shared_prompt.push_str(&prompt_text);
                if p.shared_prompt.len() > max_prompt {
                    let boundary = p
                        .shared_prompt
                        .ceil_char_boundary(p.shared_prompt.len() - max_prompt);
                    p.shared_prompt = p.shared_prompt[boundary..].to_string();
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
                wall_ms,
            })
        }
        Err(e) => {
            warn!("live transcription: chunk failed: {}, skipping", e);
            // Drop our local Arc clone so the restart helper's `try_unwrap`
            // isn't racing us.
            drop(client);
            recover_from_chunk_failure(ctx).await
        }
    }
}

/// Inspect the transcription client after a chunk error: if the sidecar is
/// still running the failure was transient and we just retry; if it died,
/// try to respawn it. respawn needs `&mut TranscriptionClient`, so we
/// `try_unwrap` the inner `Arc`. When another task still holds a clone (a
/// concurrent chunk awaiting the dead sidecar's response), the unwrap fails;
/// we put the `Arc` back untouched and return `Skipped`, letting that other
/// task hit the same error and retry on the next chunk. Returns the outcome
/// the caller should propagate.
async fn recover_from_chunk_failure(ctx: &TranscriptionContext) -> TranscribeOutcome {
    let mut client_guard = ctx.transcription_client.lock().await;
    let Some(arc_client) = client_guard.take() else {
        return TranscribeOutcome::Skipped;
    };
    if arc_client.is_running() {
        // Sidecar still running — just a transient error. Put the Arc back
        // and let the caller retry.
        *client_guard = Some(arc_client);
        return TranscribeOutcome::Skipped;
    }

    warn!("sidecar process died — attempting restart");
    match Arc::try_unwrap(arc_client) {
        Ok(mut client) => match client.respawn().await {
            Ok(()) => {
                info!("sidecar restarted successfully after transcription failure");
                *client_guard = Some(Arc::new(client));
                let _ = ctx.app_handle.emit(
                    "live-transcription-warning",
                    LiveTranscriptionWarningEvent {
                        message: "Transcription engine restarted".into(),
                    },
                );
                TranscribeOutcome::Skipped
            }
            Err(restart_err) => {
                error!("sidecar restart failed: {}", restart_err);
                // Put the client back so other tasks can see it as "not
                // running" and try again.
                *client_guard = Some(Arc::new(client));
                TranscribeOutcome::SidecarDead
            }
        },
        Err(still_shared) => {
            // Another task still holds the Arc. Put it back untouched; that
            // task will hit the same error and we'll try again on the next
            // chunk.
            *client_guard = Some(still_shared);
            debug!("sidecar restart skipped: client still held by another chunk task");
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
    counters: &Arc<StdMutex<SessionCounters>>,
    prompt: &Arc<StdMutex<PromptState>>,
) -> TranscribeOutcome {
    let outcome = transcribe_chunk(ctx, input, chunk_index, accumulated_text, prompt).await;

    if let TranscribeOutcome::Success(ref result) = outcome {
        // Short critical section — no await held.
        let (total_chunks, total_audio_seconds, cap_fired_total) = {
            let mut s = counters.lock().expect("counters mutex poisoned");
            s.total_chunks += 1;
            s.total_audio_seconds += result.chunk_duration;
            s.total_wall_ms = s.total_wall_ms.saturating_add(result.wall_ms);
            // Track the latest emitted chunk's session-time end so the
            // status command can compute live lag against the wall clock.
            // Pressure event already reported per-chunk lag; this is for the
            // polled-status surface used by StatusPopover.
            let chunk_end = result.event.audio_offset_seconds + result.chunk_duration;
            s.latest_completed_audio_offset_seconds = Some(chunk_end);
            (s.total_chunks, s.total_audio_seconds, s.cap_fired_total)
        };
        prompt
            .lock()
            .expect("prompt mutex poisoned")
            .last_transcription_at = Some(Instant::now());

        // Compute lag for the status broadcast below — same formula as the
        // pressure event but read from ctx so emit_status can stay engine-
        // and chunk-agnostic.
        let session_time_now =
            ctx.session_offset_base_seconds + ctx.session_start_instant.elapsed().as_secs_f32();
        let chunk_end_session_time = result.event.audio_offset_seconds + result.chunk_duration;
        let lag_seconds = Some((session_time_now - chunk_end_session_time).max(0.0));

        let _ = ctx
            .app_handle
            .emit("live-transcription-segment", &result.event);
        emit_status(
            &ctx.app_handle,
            LiveTranscriptionPhase::Running,
            total_chunks,
            total_audio_seconds,
            lag_seconds,
            cap_fired_total,
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
    restart_intent_inbox: tauri::State<'_, RestartIntentInbox>,
    app_handle: AppHandle,
    mut config: LiveTranscriptionConfig,
) -> Result<LiveTranscriptionStartResult, CommandError> {
    let mut guard = live_state.lock().await;

    // Check if already running
    if let Some(ref runtime) = *guard {
        if runtime.controller.is_running() {
            return Err(CommandError::InvalidInput {
                message: "live transcription is already running".into(),
            });
        }
    }

    // Validate config values. Use `is_finite` alongside the sign checks
    // because NaN comparisons all return false — raw `<= 0.0` silently
    // admits NaN payloads which would later panic in sample-count math.
    if config.silence_duration_ms == 0 {
        return Err(CommandError::InvalidInput {
            message: "silence_duration_ms must be > 0".into(),
        });
    }
    if !config.max_chunk_seconds.is_finite() || config.max_chunk_seconds <= 0.0 {
        return Err(CommandError::InvalidInput {
            message: "max_chunk_seconds must be finite and > 0".into(),
        });
    }
    if !config.backfill_seconds.is_finite() || config.backfill_seconds < 0.0 {
        return Err(CommandError::InvalidInput {
            message: "backfill_seconds must be finite and >= 0".into(),
        });
    }
    if let Some(decay) = config.prompt_decay_silence_seconds {
        if !decay.is_finite() || decay < 0.0 {
            return Err(CommandError::InvalidInput {
                message: "prompt_decay_silence_seconds must be finite and >= 0".into(),
            });
        }
    }
    // Validate MP3 bitrate now so a misconfigured session doesn't fail late,
    // after the buffer has been drained into the session WAV with no recovery.
    let will_write_mp3 = config
        .audio_export_format
        .unwrap_or(AudioExportFormatDto::Mp3)
        .is_mp3();
    if will_write_mp3 {
        if let Some(kbps) = config.mp3_bitrate {
            yapstack_audio::export::validate_mp3_bitrate(kbps).map_err(|e| {
                CommandError::InvalidInput {
                    message: e.to_string(),
                }
            })?;
        }
    }

    // Pre-flight: restart any silently-stalled capture streams before we
    // read backfill or spawn the loop. Without this, the first dictation
    // after a long idle (device change, Bluetooth drop, OS sleep) transcribes
    // whatever stale audio happens to be in the ring buffer.
    let preflight_source: CaptureSource = config.source.clone().into();
    preflight_stream_health(audio_state.inner(), &preflight_source, &app_handle).await?;

    // Resuming forces backfill to 0 — new audio is captured fresh, not
    // dredged from the ring buffer.
    if config.resume.is_some() {
        config.backfill_seconds = 0.0;
    }
    let session_offset_base_seconds = config
        .resume
        .as_ref()
        .map(|r| r.offset_base_seconds.max(0.0))
        .unwrap_or(0.0);

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

    // Set up streaming WAV writer if session_id is provided.
    let session_wav_state =
        if let Some(ref session_id) = config.session_id {
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

            // Persist the chosen audio dir before writing anything. This way
            // reconciliation on next launch can scan it for orphans even if
            // the run dies between WAV finalize and `insert_audio_part_row`.
            // Also register it as trusted so playback works while the run is
            // still in progress.
            if let Some(db_path_state) = app_handle.try_state::<crate::DbPath>() {
                crate::db::register_audio_save_location(db_path_state.as_path(), &audio_dir);
            }
            crate::register_trusted_audio_dir(&app_handle, &audio_dir);

            // Each recording run produces its own part file. Resume passes the
            // next index; fresh sessions always start at 0.
            let part_index = config.resume.as_ref().map(|r| r.part_index).unwrap_or(0);
            let wav_path = audio_dir.join(format!("{session_id}.{part_index}.wav"));

            let sample_rate = {
                let manager = audio_state.lock().await;
                manager
                    .mic_buffer_info()
                    .map(|i| i.sample_rate)
                    .or_else(|| manager.system_buffer_info().map(|i| i.sample_rate))
                    .unwrap_or(48000)
            };
            let writer = yapstack_audio::SessionWavWriter::new(wav_path.clone(), sample_rate)
                .map_err(|e| CommandError::Internal {
                    message: format!("failed to create session WAV: {e}"),
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
                part_index,
                flush_count: 0,
                wav_sample_rate: sample_rate,
                persist_audio_part: config.persist_audio_part,
            })
        } else {
            None
        };

    let (stop_tx, stop_rx) = oneshot::channel();

    // Channel for RestartIntents from the device broker. The sender is
    // installed in the managed inbox so the broker can find it; the
    // receiver is consumed by the live loop. Replaces any prior sender
    // (which would only exist if a previous session crashed without
    // clearing the inbox in stop_live_transcription).
    let (restart_intent_tx, restart_intent_rx) =
        tokio::sync::mpsc::unbounded_channel::<RestartIntent>();
    {
        let mut inbox_guard = restart_intent_inbox
            .inner()
            .lock()
            .expect("restart-intent inbox poisoned");
        *inbox_guard = Some(restart_intent_tx);
    }

    let audio_state_clone = audio_state.inner().clone();
    let transcription_state_clone = transcription_state.inner().clone();

    // Align config backfill with the clamped value so WAV writer and transcript
    // cursor share the same time origin (prevents timestamp drift on playback).
    config.backfill_seconds = effective_backfill_seconds;

    // Capture session_id before config is moved into TranscriptionContext
    let controller_session_id = config.session_id.clone();

    // Extract the transcription client only after all fallible setup above
    // succeeds. This avoids losing the client on early-return setup errors.
    // The client is already `Arc<TranscriptionClient>` in shared state —
    // take() moves the Arc into the live context so other commands that
    // `lock + as_ref().cloned()` see `None` while the live loop owns it.
    let extracted_client = {
        let mut client_guard = transcription_state.lock().await;
        client_guard.take().ok_or(CommandError::NotInitialized {
            message: "transcription client not initialized".into(),
        })?
    };

    let vocab_hints = Arc::new(Mutex::new(
        config.vocabulary_hints.clone().unwrap_or_default(),
    ));

    let session_start_instant = Instant::now();
    // Resolve the engine profile once at session start using the live
    // client. After this point, neither the live loop nor `transcribe_chunk`
    // need to re-read `client.engine()` to discover engine-specific
    // behavior — they consult `ctx.engine_profile` instead.
    let mut profile = profile_for(extracted_client.engine(), &config);
    // Fold the resolved sidecar engine info into the profile so per-chunk
    // pressure events can report which accel and variant the session ran
    // on without crossing back through Tauri state. `engine_info()` was
    // populated by `init_transcription_client` at spawn time.
    if let Some(info) = extracted_client.engine_info() {
        profile.accel = info.accel;
        profile.variant = info.model_dir.as_deref().and_then(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        });
    }
    let engine_profile = Arc::new(profile);
    // Logged at INFO so a default-level log capture pinned to a stalled
    // session always shows which engine + tuning + accel were active.
    info!(
        engine = engine_profile.engine_name,
        silence_ms = engine_profile.vad_tuning.silence_duration.as_millis() as u64,
        poll_ms = engine_profile.vad_tuning.poll_interval.as_millis() as u64,
        pre_roll_ms = engine_profile.vad_tuning.pre_roll.as_millis() as u64,
        max_chunk_secs = engine_profile.vad_tuning.max_chunk_duration.as_secs_f32(),
        uses_initial_prompt = engine_profile.uses_initial_prompt,
        accel = engine_profile.accel.as_deref(),
        variant = engine_profile.variant.as_deref(),
        session_id = ?config.session_id,
        offset_base_secs = session_offset_base_seconds,
        "live transcription engine profile resolved"
    );
    let ctx = TranscriptionContext {
        transcription_client: Arc::new(Mutex::new(Some(extracted_client))),
        shared_transcription_state: transcription_state_clone,
        app_handle,
        config,
        bridged_prompt: Arc::new(Mutex::new(String::new())),
        vocabulary_hints: vocab_hints.clone(),
        session_offset_base_seconds,
        session_start_instant,
        engine_profile,
    };

    let counters = Arc::new(StdMutex::new(SessionCounters {
        total_chunks: 0,
        total_audio_seconds: 0.0,
        cap_fired_total: 0,
        total_wall_ms: 0,
        latest_completed_audio_offset_seconds: None,
    }));

    let task_handle = tokio::spawn({
        let ctx_guard = ctx.clone();
        let counters_for_loop = counters.clone();
        async move {
            let result = AssertUnwindSafe(live_transcription_loop(
                audio_state_clone,
                ctx,
                stop_rx,
                restart_intent_rx,
                session_wav_state,
                counters_for_loop,
            ))
            .catch_unwind()
            .await;

            // Always return the transcription client to shared state, even
            // after a panic. Both private and shared state hold
            // `Arc<TranscriptionClient>`, so the Arc moves straight back —
            // no `try_unwrap` dance, and a chunk task that leaked a clone
            // won't strand shared state empty.
            {
                let mut private_guard = ctx_guard.transcription_client.lock().await;
                if let Some(arc_client) = private_guard.take() {
                    let mut shared_guard = ctx_guard.shared_transcription_state.lock().await;
                    *shared_guard = Some(arc_client);
                    debug!("returned transcription client to shared state");
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
                        lag_seconds: None,
                        cap_fired_total: 0,
                    },
                );
            }
        }
    });

    *guard = Some(LiveTranscriptionRuntime {
        controller: LiveTranscriptionController {
            task_handle,
            stop_tx: Some(stop_tx),
            session_id: controller_session_id,
            effective_start_epoch_ms,
            counters,
            session_start_instant,
            session_offset_base_seconds,
        },
        vocabulary_hints: vocab_hints,
    });

    Ok(LiveTranscriptionStartResult {
        effective_start_epoch_ms,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn stop_live_transcription(
    live_state: tauri::State<'_, LiveTranscriptionState>,
    restart_intent_inbox: tauri::State<'_, RestartIntentInbox>,
) -> Result<(), CommandError> {
    let mut guard = live_state.lock().await;

    // Clear the restart-intent inbox first so the broker can't post into
    // a soon-to-be-dropped receiver. The loop's select! will hit the
    // stop_rx branch and exit; any drained-but-unprocessed intent is
    // discarded along with the receiver.
    {
        let mut inbox_guard = restart_intent_inbox
            .inner()
            .lock()
            .expect("restart-intent inbox poisoned");
        *inbox_guard = None;
    }

    match guard.take() {
        Some(mut runtime) => {
            if let Some(tx) = runtime.controller.stop_tx.take() {
                let _ = tx.send(());
            }
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

    match &*guard {
        Some(runtime) => {
            let c = &runtime.controller;
            let (chunks_processed, total_audio_seconds, lag_seconds, cap_fired_total) =
                c.snapshot();
            let phase = if c.is_running() {
                LiveTranscriptionPhase::Running
            } else {
                LiveTranscriptionPhase::Stopped
            };
            Ok(LiveTranscriptionStatus {
                phase,
                chunks_processed,
                total_audio_seconds,
                error_message: None,
                session_id: c.session_id.clone(),
                effective_start_epoch_ms: Some(c.effective_start_epoch_ms),
                lag_seconds,
                cap_fired_total,
            })
        }
        None => Ok(LiveTranscriptionStatus {
            phase: LiveTranscriptionPhase::Stopped,
            chunks_processed: 0,
            total_audio_seconds: 0.0,
            error_message: None,
            session_id: None,
            effective_start_epoch_ms: None,
            lag_seconds: None,
            cap_fired_total: 0,
        }),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn update_vocabulary_hints(
    live_state: tauri::State<'_, LiveTranscriptionState>,
    hints: String,
) -> Result<(), CommandError> {
    let guard = live_state.lock().await;
    match &*guard {
        Some(runtime) => {
            let mut vocab = runtime.vocabulary_hints.lock().await;
            *vocab = hints;
            Ok(())
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            silence_duration_ms: 800,
            max_chunk_seconds: 30.0,
            backfill_seconds: 0.0,
            source: CaptureSourceDto::MicOnly,
            mix_config: None,
            language: None,
            prompt_context_chars: None,
            prompt_decay_silence_seconds: None,
            session_id: None,
            persist_audio_part: true,
            audio_save_location: None,
            audio_export_format: None,
            mp3_bitrate: None,
            diarization: false,
            vocabulary_hints: None,
            resume: None,
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
        let t = profile_for(EngineKind::Whisper, &cfg).vad_tuning;
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
        let t = profile_for(EngineKind::Parakeet, &cfg).vad_tuning;
        assert_eq!(
            t.silence_duration,
            Duration::from_millis(500),
            "Parakeet silence window should be 500ms for meeting-style chunking \
             (FluidAudio uses 750ms; 200ms over-dispatched on fast dialogue and \
             contributed to the stall the cap commit was bounding)"
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
        let tuning = profile_for(EngineKind::Parakeet, &cfg).vad_tuning;
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
        let tuning = profile_for(EngineKind::Parakeet, &cfg).vad_tuning;

        let sr: u32 = 48_000;
        // Frame counts derived from the active tuning so this regression
        // doesn't have to be retuned every time the silence/pre-roll
        // defaults shift. We need:
        //   - loud region long enough to be a real chunk (≥ 1 s),
        //   - gap longer than `silence_duration` so the first chunk fires,
        //   - gap shorter than `silence_duration + pre_roll + small slack`
        //     so the second onset's pre-roll could (without the clamp)
        //     reach back into the first chunk's tail.
        let frame_secs = super::super::silero_vad::FRAME_DURATION_SECS;
        let frames_for = |ms: u64| -> usize {
            ((Duration::from_millis(ms).as_secs_f32() / frame_secs).ceil() as usize).max(1)
        };
        let loud_frames = frames_for(1_000); // 1.0 s of speech
                                             // One frame past the silence threshold — guarantees the silence
                                             // break fires while keeping the gap short enough that pre-roll
                                             // would overlap without the clamp.
        let silence_frames = frames_for(tuning.silence_duration.as_millis() as u64);
        let gap_frames = silence_frames + 1;

        let mut probs: Vec<f32> = Vec::with_capacity(loud_frames * 2 + gap_frames);
        probs.extend(std::iter::repeat_n(0.90, loud_frames));
        probs.extend(std::iter::repeat_n(0.10, gap_frames));
        probs.extend(std::iter::repeat_n(0.90, loud_frames));

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
        s.silero.reset();

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
