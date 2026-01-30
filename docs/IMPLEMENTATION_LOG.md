# Implementation Log

What was built, in what order, and key decisions made along the way.

---

## Phase 1 (Prior — Foundation)

Established the project skeleton and core audio infrastructure.

### What was built
- **Workspace structure**: 5 crates (`yapstack-common`, `yapstack-audio`, `yapstack-transcription`, `yapstack-sidecar`, `yapstack-desktop`)
- **yapstack-common**: `AppConfig`, `AudioConfig`, `TranscriptionConfig`, domain types (`CaptureState`, `AudioSource`, `DeviceType`, `PermissionStatus`, etc.)
- **yapstack-audio core**: `AudioRingBuffer` (lock-free SPSC), `MicrophoneCapture`, `SystemAudioCapture` (platform-abstracted), `AudioManager`, device enumeration, error types
- **yapstack-desktop**: Tauri app skeleton with tray icon, audio commands (list devices, start/stop capture, snapshots, buffer info), DTO layer with specta type generation
- **Frontend**: React 19 + Vite + TailwindCSS + Zustand skeleton, App component, test setup
- **macOS entitlements**: audio-input + screen-capture

### Key decisions
- **16kHz mono** as default audio format (Whisper-optimal)
- **180-second ring buffer** to balance memory (~11.5 MB) with capture history
- **Lock-free SPSC ring buffer** using `UnsafeCell` + `AtomicUsize` — audio callbacks cannot block
- **Platform abstraction via enum variants** for `SystemAudioCapture` — macOS/Windows/Unavailable
- **DTO pattern** — domain types don't depend on specta; Tauri commands use separate DTO structs with `From` impls

---

## Phase 2 — Mixer, WAV Export, Capture Triggers

Added audio mixing, WAV export, and capture orchestration (instant + session modes).

### What was built

| File | Purpose |
|------|---------|
| `yapstack-audio/src/mixer.rs` | `MixConfig`, `mix_to_mono()`, `apply_gain()`, `normalize_in_place()` — pure functions, 9 tests |
| `yapstack-audio/src/export.rs` | `write_wav()`, `write_wav_to_temp()` — 16-bit PCM via hound, 4 tests |
| `yapstack-audio/src/capture.rs` | `CapturedAudio`, `SessionMark`, `CaptureResult` data types |
| `commands/capture.rs` | 4 Tauri commands: `trigger_instant_capture`, `start_session`, `end_session`, `get_session_status` |

### Modified files
- **ring_buffer.rs**: Added `samples_written()` and `snapshot_since()` for session support (4 tests)
- **error.rs**: Added `WavExport`, `NoActiveSession`, `SessionAlreadyActive`, `NoBufferAvailable` + `From<hound::Error>`, `From<io::Error>`
- **yapstack-common/types.rs**: Added `CaptureSource` enum
- **manager.rs**: Added `session_mark` field, 8 new methods for capture extraction and session management (4 tests)
- **audio.rs** (DTOs): Added `CaptureSourceDto`, `CaptureResultDto`, `MixConfigDto`, `SessionStatusDto` + `From` impls

### Dependencies added
- `hound = "3.5"` — WAV file I/O
- `tempfile = "3"` — temp file management for WAV export

### Key decisions
- **Session tracking via write_pos snapshots** — `start_session()` records the monotonic `write_pos`, `end_session()` uses `snapshot_since()` to extract exactly the audio recorded during the session. No separate buffer needed.
- **Silent truncation for long sessions** — if a session exceeds buffer capacity, `snapshot_since()` clamps to capacity and a warning is logged. `CaptureResult.duration_seconds` reflects actual captured duration.
- **f32 clamping before i16 conversion** — `clamp(-1.0, 1.0)` before WAV export prevents overflow
- **Temp files persist** — `write_wav_to_temp()` uses `tempfile::Builder` with `.keep()` so files outlive the function call. Cleanup is the caller's responsibility.

---

## Phase 3 — Whisper Sidecar Integration

Added transcription via a sidecar process architecture: model management, JSON-line IPC, and Tauri commands.

### What was built

| File | Purpose |
|------|---------|
| `yapstack-transcription/src/model.rs` | `ModelSize`, `ModelManager` — download from HuggingFace, SHA-256 verification, list/delete, 8 tests |
| `yapstack-transcription/src/whisper.rs` | `WhisperClient` — spawns sidecar, JSON-line IPC, timeout handling |
| `yapstack-sidecar/src/main.rs` | Full sidecar binary — JSON-line protocol, `TranscriptionEngine` (behind `whisper` feature), WAV loading via hound |
| `commands/transcription.rs` | 6 Tauri commands: `get_available_models`, `download_model`, `delete_model`, `transcribe_audio`, `init_whisper_client`, `shutdown_whisper_client` |
| `scripts/build-sidecar.sh` | Build script for sidecar with target triple detection |

### Modified files
- **yapstack-common/types.rs**: Added `TranscriptSegment` (with confidence), `SidecarRequest`, `SidecarResponse` IPC protocol types (3 tests)
- **yapstack-transcription/error.rs**: Expanded to 11 variants: added `DownloadFailed`, `ChecksumMismatch`, `SidecarError`, `SidecarNotRunning`, `Timeout`, `Io`, `Json`, `Http`
- **yapstack-sidecar/Cargo.toml**: Restructured — removed yapstack-audio/yapstack-transcription deps, added `whisper` feature flag, direct deps on `hound`, `tracing-subscriber`
- **tauri.conf.json**: Added `externalBin` for sidecar bundling
- **apps/desktop/src-tauri/Cargo.toml**: Added `yapstack-transcription` dependency

### Dependencies added
- `reqwest = { version = "0.12", features = ["stream"] }` — HTTP client for model downloads
- `sha2 = "0.10"` — checksum verification
- `futures-util = "0.3"` — streaming download
- `tracing-subscriber = { version = "0.3", features = ["env-filter"] }` — sidecar logging to stderr

### Key decisions
- **Sidecar process architecture** — Whisper runs in a separate binary (`yapstack-sidecar`), not in the main app process. Benefits: crash isolation, heavy native dep (`whisper.cpp` + cmake) doesn't affect main app build, can be updated independently.
- **JSON-line IPC protocol** — Simple, debuggable. Each message is a tagged JSON object on one line. Serde's `#[serde(tag = "type")]` provides clean serialization.
- **whisper feature flag on sidecar** — Without cmake, the sidecar still compiles but responds with error messages. This keeps `cargo build --all` working on machines without cmake.
- **Model download from HuggingFace** — Uses `ggerganov/whisper.cpp` ggml models. Downloads to `app_data_dir/models/` with `.download` temp extension, renamed atomically on completion.
- **whisper-rs v0.15 API** — Uses `state.get_segment(i)` (returns `Option<WhisperSegment>`), `segment.to_str()`, `segment.start_timestamp()`/`end_timestamp()` (centiseconds), `segment.no_speech_probability()` for confidence.
- **Sidecar logging to stderr** — stdout is reserved for IPC. `tracing-subscriber` writes to stderr with env-filter support.
- **Sidecar binary naming** — `yapstack-sidecar-{target-triple}` in `apps/desktop/src-tauri/binaries/`. Tauri's `externalBin` config handles the target triple suffix automatically.

### whisper-rs API notes (v0.15.1)
The whisper-rs v0.15 API differs from older examples online:
- `full_n_segments()` returns `c_int` directly (not `Result`)
- Segment access via `state.get_segment(i) -> Option<WhisperSegment>` (not `full_get_segment_text`)
- `WhisperSegment` has `.to_str()`, `.start_timestamp()`, `.end_timestamp()`, `.no_speech_probability()`
- Timestamps are in **centiseconds** (multiply by 10 for milliseconds)

---

## Test Coverage Summary

| Crate | Tests | Notes |
|-------|-------|-------|
| yapstack-audio | 92 passed, 4 ignored | Ring buffer (incl. rms_energy_since), mixer, manager (incl. position API, synthetic buffer tests, stream health), device, export (incl. SessionWavWriter). Ignored tests need hardware. |
| yapstack-common | 18 passed | Config, types serde, IPC protocol, audio deinterleave |
| yapstack-transcription | 10 passed | Model management unit tests, VAD model management |
| yapstack-sidecar | 19 passed | Hallucination filter (`should_include_segment`), repetition detection, punctuation normalization |
| yapstack-desktop | 48 passed | CommandError, MixConfigDto sanitization, trim_leading_silence, capture_source_from_dto, VAD state machine, prompt decay |
| frontend (vitest) | 294 passed | 10 lib + 9 component + 2 hook test files. See Phases 18, 21, 22. |
| **Total** | **481 passed** | |

---

## Phase 4 — Capture Source Fix + Sidecar Resampling

Fixed two bugs in the capture-to-transcription pipeline.

### Bug 1: System-audio-only capture produces "no buffer" error

**Root cause**: `start_capture` Tauri command took an `include_system_audio: bool` parameter. It only had two paths: `true` → `start_all()` (mic first, system optional), `false` → `start_mic()` only. There was no path to start system audio without also starting the mic. When the user selected "System Only", the system buffer was never created, so `trigger_instant_capture()` / `end_session()` returned `AudioError::NoBufferAvailable`.

**Fix**: Replaced `include_system_audio: bool` with `capture_source: CaptureSourceDto` across the stack.

| File | Change |
|------|--------|
| `yapstack-audio/src/manager.rs` | Added `start_capture(source, mic_device_name)` dispatcher that routes `MicOnly` → `start_mic()`, `SystemOnly` → `start_system_audio()` (hard error if unavailable), `Mixed` → `start_all()` |
| `commands/audio.rs` | Replaced `include_system_audio: bool` param with `capture_source: CaptureSourceDto` |
| `stores/appStore.ts` | Removed `includeSystemAudio` from `Settings`, pass `captureSource` to `startCapture` |
| `components/AudioPanel.tsx` | Removed redundant "Include System Audio" toggle and `Switch` import |
| `src-tauri/src/lib.rs` | Tray handler uses `start_capture(MicOnly, None)` instead of `start_mic(None)` |

### Bug 2: Transcription produces garbled output ("[silence] [imitates sound]")

**Root cause**: Sample rate mismatch. `start_mic()` overrides `config.sample_rate` from 16kHz to the device's native rate (typically 48kHz on macOS). Audio is captured and exported to WAV at 48kHz. The sidecar passed these 48kHz samples directly to Whisper, which expects 16kHz mono. Whisper interpreted the audio as 3x faster than reality, producing garbled output. The WAV file played fine in media players because they honor the 48kHz header.

**Fix**: Added audio preprocessing in the sidecar before Whisper inference.

| File | Change |
|------|--------|
| `yapstack-sidecar/src/main.rs` | Added `to_mono()` (channel averaging) and `resample()` (linear interpolation) functions. After reading the WAV, sidecar converts to mono and resamples to 16kHz before calling `state.full()`. |

### Key decisions
- **Resample in the sidecar, not during capture** — The WAV file should contain audio at the device's native sample rate so it plays back correctly in any media player. The 16kHz requirement is specific to Whisper, so the conversion belongs at the transcription boundary.
- **Linear interpolation resampling** — Simple, no external dependencies, sufficient quality for speech audio. Handles arbitrary sample rate ratios (not just integer multiples).
- **Hard error for SystemOnly when unavailable** — Unlike `start_all()` which silently falls back to mic-only when system audio fails, `start_capture(SystemOnly)` returns `PlatformNotSupported` because the user explicitly chose system-only capture.

---

## Phase 5 — System Audio Sample Rate / Channel Mismatch Fix

Fixed system audio playing back slowed down (~0.5x) and distorted when captured alongside mic audio.

### Root cause

The mic is typically 48kHz **mono** (1ch), while the system output device is 48kHz **stereo** (2ch). `start_mic()` was writing the mic's config into `self.config` (48kHz, 1ch). All downstream code — `extract_captured_audio()`, `trigger_instant_capture()`, `end_session()` — used `self.config` to compute sample counts, WAV headers, and duration. So stereo system audio got a mono WAV header — the sidecar interpreted it at half the frame rate — resulting in 0.5x playback.

### What changed

| File | Change |
|------|--------|
| `yapstack-audio/src/mixer.rs` | Added `deinterleave_to_mono()` — converts interleaved multi-channel audio to mono by averaging channels per frame. 4 tests. Updated `mix_to_mono` doc: inputs must already be mono. |
| `yapstack-audio/src/error.rs` | Added `SampleRateMismatch { mic_rate, system_rate }` variant |
| `yapstack-audio/src/manager.rs` | `start_mic()` no longer mutates `self.config`; buffer uses `device_config` directly. `extract_captured_audio()` reads each buffer's `sample_rate()` / `channels()` and deinterleaves to mono. `trigger_instant_capture()` and `end_session()` rewritten to use mono-normalized `CapturedAudio`. WAV export always uses `channels = 1`. Mixed-source paths guard against sample rate mismatch. Deleted `extract_for_source()` (folded into `trigger_instant_capture`). |

### Key decisions
- **Per-buffer format independence** — Each ring buffer carries its own `sample_rate` and `channels`. Extraction methods read these from the buffer instead of relying on a shared `self.config`. This prevents one device's format from leaking into another device's audio path.
- **Deinterleave at extraction boundary** — Multi-channel data is converted to mono immediately after snapshot, before any mixing or WAV export. All downstream code operates on mono data.
- **SampleRateMismatch error** — Mixed-source capture fails explicitly if mic and system buffers have different sample rates, rather than producing garbled audio.
- **No changes to export, ring buffer, or sidecar** — The fix is entirely within the manager's extraction logic and the new `deinterleave_to_mono` utility.

---

## Phase 6 — Live Transcription

Added real-time live transcription with voice activity detection (VAD), per-source audio tracking, backfill, and prompt context.

### What was built

| File | Purpose |
|------|---------|
| `commands/live_transcription.rs` | Live transcription controller, VAD state machine, per-source chunk processing, backfill support. 3 Tauri commands: `start_live_transcription`, `stop_live_transcription`, `get_live_transcription_status` |
| `yapstack-audio/src/capture.rs` | `BufferPositions`, `SeparateExtraction` types for position-based extraction |
| `yapstack-audio/src/manager.rs` | `buffer_positions()`, `extract_since()`, `extract_sources_since()`, `peek_energy_rms()` — position-based APIs for live transcription |
| `yapstack-common/src/audio.rs` | `deinterleave_to_mono()` — canonical shared implementation |

### Key decisions
- **Per-source VAD** — mic and system audio tracked independently with separate energy thresholds and silence state machines. Each source produces its own chunks.
- **Backfill** — `backfill_seconds` captures audio from before the live session started, so the first chunk includes context.
- **Prompt context** — Prior transcript text (up to `prompt_context_chars`) is fed as `initial_prompt` to Whisper for better continuity.

---

## Phase 7 — Second-Pass Refactor

Systematic code review by two senior engineer agents (React + Rust) found 33 actionable items. All were implemented.

### What was fixed

**Bugs & race conditions (6 items)**:
- `setLivePhase("Stopped")` async state race — re-reads `selectedSessionId` inside `.then()` callback
- `onLiveSegment` stale `activeSessionId` guard after awaits
- `clearAllSessions` guard against active recording (refuses with toast)
- `wait_for_response` returning Progress as final response (check before return)
- `init_whisper_client` / `delete_model` hold model manager lock during async ops (extract paths first, drop lock)

**Error handling (6 items)**:
- `updateSettings` capture restart `.catch()` with toast
- `stopActiveSession` bare `catch {}` → `console.error`
- `autoSetup` `.refreshDevices()` `.catch()`
- `capture_history_seconds` input validation (positive + finite)
- `BufferDetail` division by zero guard
- Model download temp file cleanup on rename failure

**React patterns (5 items)**:
- `ModelSection` setState during render → `useEffect`
- `StatusBar` inline source labels → shared `SOURCE_LABELS_FULL` constant
- `deinterleave_to_mono` canonical in `yapstack_common::audio`, mixer wrapper `pub(crate)`
- `TranscriptionTab` option arrays hoisted to module scope
- Removed unused `_config` parameter from `MicrophoneCapture::start`

**Rust soundness (4 items)**:
- Removed `unsafe impl Sync for SendStream` (only `Send` needed)
- `debug_assert` for incomplete audio frames in `deinterleave_to_mono`
- `warn!()` log for `extract_since` silent `None` on sample rate mismatch
- `MixConfigDto::sanitized()` validates NaN/infinity/negative gains at command boundary

**Accessibility (3 items)**:
- `aria-label="Back to sessions"` on SettingsPanel back button
- `sr-only` Recording label in SessionListItem
- `aria-label="Settings"` on SessionSidebar settings button

---

## Phase 8 — Transcription Configurability

Added user-configurable transcription parameters and prompt context windowing.

### What was built

| File | Change |
|------|--------|
| `commands/live_transcription.rs` | Added `prompt_context_chars` to `LiveTranscriptionConfig` (default 350). Prior transcript text fed as `initial_prompt` to Whisper for continuity. |
| `yapstack-sidecar/src/main.rs` | `TranscriptionEngine::transcribe()` accepts `initial_prompt` parameter. Added Whisper params: `suppress_blank`, `suppress_nst`, `no_context`, `temperature_inc(0.0)`. Enabled flash attention via `ctx_params.flash_attn(true)`. |
| `yapstack-common/types.rs` | Added `initial_prompt: Option<String>` to `SidecarRequest::Transcribe` |
| `stores/appStore.ts` | Added `promptContextChars` to Settings. Settings version bumped to 4 with migration. |
| `components/settings/TranscriptionTab.tsx` | UI for prompt context chars setting |

### Key decisions
- **Prompt context at Whisper level** — Rather than concatenating audio chunks, feed prior text as `initial_prompt`. Simpler, no audio manipulation, Whisper uses it for vocabulary/style priming.
- **350 char default** — Enough for ~2 sentences of context without overwhelming the prompt budget.

---

## Phase 9 — Backfill & Reliability Improvements

Systematic improvements to the backfill transcription flow, concurrent processing, and error handling.

### What was built

| File | Change |
|------|--------|
| `commands/live_transcription.rs` | Refactored to shared `TranscriptionContext` struct. Backfill runs concurrently with live VAD via `tokio::select!`. Added `trim_leading_silence()` (50ms windows, 200ms pad). Added `is_backfill` flag to `LiveSegmentEvent`. Emits `backfill-complete` event. Removed `ProcessingBackfill` phase. Added `ChunkResult` return type. Interleaved backfill processing (window 0 for all sources, then window 1). Skip entirely silent chunks. |
| `stores/appStore.ts` | Replaced `backfillPendingSeconds: number | null` with `backfillActive: boolean`. Added `onBackfillComplete()` handler. `onLiveSegment` wrapped in serialization queue (`segmentQueueTail`) to prevent concurrent DB write races. Re-reads `activeSessionId` after awaits to guard against navigation races. Added `console.error` for all catch blocks (replaced silent failures). Guard against `clearAllSessions` during active recording. Capture restart skipped during active live transcription. |
| `hooks/useLiveTranscriptionEvents.ts` | Added `backfill-complete` event listener |
| `yapstack-sidecar/src/main.rs` | Added `should_include_segment()` hallucination filter. Filters: empty text, special tokens, low confidence (< 0.4), known patterns at marginal confidence (< 0.6). Tests for filter function. |
| `yapstack-audio/src/manager.rs` | `start_all()` surfaces degradation error message when system audio fails (instead of silent fallback). `extract_since()` logs warning on sample rate mismatch. |

### Key decisions
- **Concurrent backfill + live** — Backfill processes historical audio in a separate tokio task while the live VAD loop runs simultaneously. No blocking. `tokio::select!` waits for either stop signal or backfill completion.
- **Serialization queue** — `enqueueSegmentWork()` chains promises to ensure only one `onLiveSegment` handler runs at a time. Prevents interleaved reads/writes to `activeSessionSegments` from concurrent backfill and live events.
- **Interleaved backfill** — Process window 0 for all sources, then window 1, etc. Produces chronologically ordered output instead of all mic chunks then all system chunks.
- **Silence trimming** — `trim_leading_silence()` scans in 50ms windows, keeps 200ms pad before first detected energy to avoid clipping speech onset. Entirely silent chunks are skipped.
- **Hallucination filtering in sidecar** — Filtering happens at the transcription boundary, not in the frontend. Known Whisper failure modes (repeating "thank you", "[BLANK_AUDIO]") are caught before they reach the UI.
- **Degradation surfacing** — `start_all()` now sets `error_message` when system audio fails instead of silently falling back. Frontend can display "Mixed mode degraded to mic-only".

---

## Phase 10 — UI Rework

Complete frontend redesign: notes-first navigation model, side-by-side editing, audio playback with waveform, rich text notes, folders, drag-and-drop, search, and multiple polish fixes.

### Navigation & Layout Overhaul

| File | Purpose |
|------|---------|
| `App.tsx` | Root layout with `ThemeProvider` (light/dark/system) |
| `AppLayout.tsx` | Main layout: `AppSidebar` + content area + `SearchCommand` + `AIChatPanel` |
| `AppSidebar.tsx` | Left sidebar: folder tree, create session/note buttons, filter (all/pinned/folder), settings |
| `NoteCardList.tsx` | Grid of `NoteCard` components for the note-list view |
| `NoteCard.tsx` | Draggable session/note card with pin badge, folder indicator, context menu |
| `FolderItem.tsx` | Sidebar folder entry with rename/delete context menu, drop target for drag-and-drop |
| `SearchCommand.tsx` | Cmd+K palette via `cmdk` — fuzzy search sessions and notes |
| `AIChatPanel.tsx` | Bottom drawer (via `vaul`) placeholder for AI assistant |
| `RecordingBeacon.tsx` | Pulsing red dot indicator during active recording |

**Navigation model changed**: Views renamed from `"sessions" | "session-detail" | "settings"` to `"note-list" | "note-detail" | "settings"`. `ListFilter` added for filtering by all/pinned/folder.

### Note Detail View

| File | Purpose |
|------|---------|
| `NoteDetailView.tsx` | Orchestrates 4 layout modes: active recording, completed transcription (split pane), manual note, fallback |
| `SessionHeaderV2.tsx` | Header: back button, editable title, source/recording badges, duration, segment count, actions dropdown (delete) |
| `NoteEditor.tsx` | Tiptap rich text editor with formatting toolbar (Bold, Italic, H1, H2, Bullet List, Ordered List, Blockquote, Code), debounced autosave, version snapshots on blur |
| `NoteHistoryPanel.tsx` | Collapsible panel showing note version history with restore |

### Audio Playback & Waveform

| File | Purpose |
|------|---------|
| `AudioPlayer.tsx` | HTML5 `<audio>` player with play/pause, seek slider, speed control (0.5x–2x), time display |
| `commands/capture.rs` | Added `export_session_wav`, `delete_session_wav` commands |
| `lib.rs` | Custom `audio-stream://` URI scheme protocol with HTTP range request support for audio seeking |

### Editable Segments

| File | Purpose |
|------|---------|
| `EditableSegment.tsx` | Segment bubble with click-to-edit (inline textarea), context menu (Edit, Copy, Hide, Delete), visual indicators for low confidence, edited, and hidden states |
| `ChatBubble.tsx` | Read-only variant for active recording sessions (later merged into `EditableSegment` via `readOnly` prop) |
| `ChatView.tsx` | Segment list with auto-scroll, playback-synced active segment highlighting (`ring-2 ring-primary shadow-md scale-[1.02]`) |

### Resizable Panels

| File | Purpose |
|------|---------|
| `ui/resizable.tsx` | Wraps `react-resizable-panels` (`Group` → `ResizablePanelGroup`, `Panel` → `ResizablePanel`, `Separator` → `ResizableHandle` with grip icon) |

### New UI Primitives (shadcn)

Added: `command.tsx`, `context-menu.tsx`, `dialog.tsx`, `dropdown-menu.tsx`, `input.tsx`, `resizable.tsx`, `sheet.tsx`, `slider.tsx`, `textarea.tsx`, `tooltip.tsx`.

### Database Changes

6 SQLite migrations (up from 1):
- v2: folders, sessions.folder_id, is_pinned, pinned_at
- v3: Segment editing (original_text, edited_at, deleted_at, hidden)
- v4: notes + note_versions tables, sessions.session_type
- v5: sessions.wav_file_path, wav_duration_seconds
- v6: sessions.sort_order, shares table

Persistence moved from `sql.js` (WebAssembly) to `tauri-plugin-sql` (native SQLite via Tauri plugin).

### Store Changes

- Settings version bumped to 5 (added `theme: ThemeMode`)
- Navigation: `currentView`, `selectedSessionId`, `listFilter` replace simple session selection
- Folders: `folders`, `loadFolders`, `createFolder`, `renameFolder`, `deleteFolder`, `moveToFolder`
- Notes: `createManualNote` for standalone notes (no transcription)
- Playback: `playbackTime`, `isPlaying`, `setPlaybackTime`, `setIsPlaying`
- Segments: `editSegmentText`, `deleteSegment`, `toggleSegmentHidden`
- `MIN_BUFFER_SECONDS` increased from 60 to 300 (5 minutes of audio capture history)

### Bug Fixes

- **Audio playback**: `convertFileSrc()` asset URLs inaccessible in dev webview; replaced with custom `audio-stream://` protocol with range request support
- **Chat scroll**: `ChatView` `ScrollArea` required flex container parent; wrapped in `<div className="flex flex-col h-full min-h-0">`
- **Buffer size**: 60s ring buffer truncated recordings; increased to 300s (~28MB per buffer at 48kHz mono)
- **Hidden segments**: Changed from removing hidden segments to greying them out (opacity + line-through)
- **Active segment highlight**: Increased visibility from `ring-2 ring-ring` to `ring-2 ring-primary shadow-md scale-[1.02]`

### Frontend Dependencies Added

- Radix: context-menu, dialog, tooltip, dropdown-menu, slider
- Tiptap: react, starter-kit, extension-placeholder, pm
- react-resizable-panels, @dnd-kit/core, @dnd-kit/sortable, @dnd-kit/utilities
- cmdk@1, vaul

### Key Decisions

- **Notes-first model** — Sessions and manual notes unified under a single list view. Every session can have attached notes (via Tiptap rich text) alongside its transcript.
- **Split pane for completed transcriptions** — Transcript on the left, notes editor on the right, with a draggable `ResizableHandle` separator. Keeps both visible simultaneously.
- **Custom audio protocol** — Rather than working around `convertFileSrc()` cross-origin issues, a dedicated `audio-stream://` protocol provides reliable WAV serving with range request support for seeking.
- **Server-side waveform peaks** — Computing peaks in Rust avoids the need for `fetch()` + `AudioContext` in the browser, which would fail cross-origin.
- **5-minute buffer** — Increased from 60s to 300s. Memory impact (~28MB per buffer) is acceptable for a desktop app. Ensures recordings >1 minute are fully captured.
- **Hidden vs deleted segments** — Hidden segments are visually greyed out (opacity 40%, line-through) but remain visible, allowing users to review and unhide. Only explicit delete removes them.

---

## Phase 11 — Unified Error Types, Streaming WAV, Bug Fixes

Systematic improvements to error handling, a fix for sessions exceeding ring buffer capacity, and a UI bug fix.

### Unified CommandError

Replaced `Result<_, String>` across all Tauri commands with a structured `CommandError` tagged union.

| File | Change |
|------|--------|
| `commands/error.rs` (NEW) | `CommandError` enum with 6 kinds: `Audio`, `Transcription`, `NotInitialized`, `InvalidInput`, `NotFound`, `Internal`. `From` impls for `AudioError`, `TranscriptionError`, `std::io::Error`. |
| `commands/audio.rs` | All 9 commands migrated from `Result<_, String>` to `Result<_, CommandError>`. Added `tracing` logs. |
| `commands/transcription.rs` | All 6 commands migrated. Locks released before async I/O. |
| `commands/capture.rs` | All 6 commands migrated. Added `tracing` logs. |
| `commands/live_transcription.rs` | All 3 commands migrated. |

### Streaming Session WAV (fix: sessions > buffer capacity)

**Problem**: Session WAV audio was capped at ring buffer capacity (180s). For longer sessions, the beginning of the audio was lost because the circular buffer overwrites old samples.

**Fix**: Stream audio to a WAV file incrementally during the live transcription loop.

| File | Change |
|------|--------|
| `yapstack-audio/src/export.rs` | New `SessionWavWriter` struct: `new()`, `write_samples()`, `finalize()`, `duration_seconds()`. 3 tests. |
| `yapstack-audio/src/lib.rs` | Re-export `SessionWavWriter` |
| `commands/live_transcription.rs` | Added `session_id: Option<String>` to `LiveTranscriptionConfig`. `SessionWavState` struct. If `session_id` set: creates WAV at `$APP_DATA_DIR/audio/{session_id}.wav`, rewinds flush positions by backfill amount, flushes every 300ms in the poll loop (same lock as energy check), final flush + finalize on stop. Emits `"session-wav-ready"` event. |
| `stores/appStore.ts` | Passes `session_id: sessionId` to config. Removed `exportSessionWav` from stop handler. Added `onSessionWavReady` action. |
| `hooks/useLiveTranscriptionEvents.ts` | Added `"session-wav-ready"` event listener. |

### WhisperClient extraction

The live transcription loop now extracts the `WhisperClient` from shared state into a private `Arc<Mutex<Option<WhisperClient>>>` for zero-contention use. The client is returned to shared state on exit (even after panics via `AssertUnwindSafe`).

### Zero-allocation energy check

| File | Change |
|------|--------|
| `yapstack-audio/src/ring_buffer.rs` | New `rms_energy_since()` — computes RMS directly on the ring buffer without allocating a `Vec<f32>`. |
| `yapstack-audio/src/manager.rs` | New `peek_energy_rms()` — delegates to `rms_energy_since()` instead of snapshot + deinterleave. New `mic_write_pos()`, `system_write_pos()` convenience methods. |

### Bug fix: session title editing

`SessionHeaderV2` `handleSaveTitle` updated the DB and sidebar (`loadSessions()`) but didn't refresh `viewSession`, causing the title to visually revert. Fixed by calling `getSession()` to refresh `viewSession` in the store after saving.

### Key decisions
- **Stream WAV during live loop** — The ring buffer is SPSC and cannot be enlarged. Instead, piggyback on the existing 300ms poll loop to periodically extract new audio. Disk writes happen outside the AudioManager lock.
- **Session WAV flush in same lock as energy check** — `extract_since()` and `peek_energy_rms()` are combined in a single lock acquisition to avoid doubling contention.
- **`export_session_wav` kept as fallback** — Still useful for short sessions or re-export scenarios where the ring buffer hasn't wrapped.
- **Unified `CommandError`** — Tagged union enables structured frontend error handling. `From` impls provide ergonomic conversion from library errors.

---

## Phase 12 — AI Chat Tool Calling & Session Intelligence

Added OpenAI-compatible tool/function calling to the AI chat, enabling the AI to directly mutate session data (title, notes, pin state) with auto-apply and undo.

### Tool Registry (`ai-tools.ts`)

Modular architecture where each tool is a self-contained `ToolDefinition` with schema, executor, and undo handler. Adding a new tool = one `registerTool()` call. No changes needed in other files.

| File | Change |
|------|--------|
| `lib/ai-tools.ts` | **New.** Tool registry with `registerTool()`, `getRegisteredTools()`, `executeTool()`, `undoToolCalls()`. Three initial tools: `update_title`, `save_to_notes`, `pin_session`. Each captures previous state for undo. |
| `lib/ai.ts` | Added `StreamEvent` type and `streamChatWithTools()` async generator. Updated `assembleTranscriptContext()` to include segment IDs (`[seg:ID ts]`). Added `marked` for markdown→HTML (replaced hand-rolled parser). `buildMessages()` accepts optional `sessionMeta`. |
| `lib/ai-prompts.ts` | Rewrote all action directives to be tool-aware. `summarize` and `meeting-minutes` instruct AI to call `update_title` + `save_to_notes`. `key-points`/`action-items` append to notes. `clean-transcript` explicitly avoids tools. Added `getSystemPromptWithToolContext()` with session metadata. Added citation instruction (`[[seg:ID]]` format). Prompts use `**bold**` not `##` headings for Tiptap compat. |
| `components/FloatingChatBar.tsx` | Replaced `streamChat()` with `streamChatWithTools()`. Handles `tool_calls` events, executes tools, prepends `[tool:name]` badge lines. Undo system with 10s toast window. Refreshes `viewSession` + sidebar + notes after tools. New props: `segments`, `onCitationClick`. |
| `components/AIChatMessage.tsx` | Parses `[tool:name]` prefix lines into badge pills. Parses `[[seg:ID]]` into clickable citation chips with tooltip. Hides empty message bubble when only badges present. New props: `segments`, `onCitationClick`. |
| `components/NoteDetailView.tsx` | Passes `segments` and `handleCitationClick` to both `FloatingChatBar` instances. Moved `segments` declaration before hooks to fix use-before-declaration. |
| `components/SessionHeaderV2.tsx` | Added `useRef`-based sync so `titleText` updates when `session.title` prop changes externally (after AI title update). |

### Key decisions

- **Single-turn tool calling** — Tools execute locally without sending results back to the model. Avoids extra API round-trip. Works because our tools are simple mutations.
- **Badge lines in message content** — `[tool:name] detail` prefix lines persist in DB. Simple, survives reload, no schema change needed.
- **`marked` over hand-rolled parser** — The custom `markdownToBasicHtml` function couldn't handle all markdown patterns (mixed headings + lists in same block, nested inline formatting). `marked` handles everything correctly.
- **Bold labels over headings in prompts** — Tiptap's StarterKit renders `<h1>`/`<h2>` disproportionately large. Prompts explicitly instruct `**bold**` for section labels.
- **Segment IDs in transcript context** — `[seg:ID timestamp]` format enables citations. The AI is instructed to cite selectively with `[[seg:ID]]` markers.

### Dependencies added

- `marked` — Markdown to HTML conversion

---

## Phase 13 — Keyboard Shortcuts

Added in-app and global keyboard shortcuts with a customizable registry.

### What was built

| File | Purpose |
|------|---------|
| `lib/shortcuts.ts` | Shortcut registry: `SHORTCUTS` array (15 shortcuts across 3 categories: Recording, Navigation, Editor), `SHORTCUT_MAP` for fast lookup, `getBinding()` with override support, `eventToBinding()`/`eventToGlobalBinding()` for capture |
| `hooks/useKeyboardShortcuts.ts` | In-app shortcuts via capture-phase `keydown` listener (mounted in `AppLayout`). Skips when input/textarea/contenteditable focused or during shortcut capture mode. 11 in-app actions. |
| `hooks/useGlobalShortcuts.ts` | Global shortcuts via `@tauri-apps/plugin-global-shortcut` (mounted in `App.tsx`). 4 global shortcuts (new session, new session with backfill, stop recording, new note). Re-registers when bindings change. `focusWindow()` brings app to front. |
| `components/settings/ShortcutsTab.tsx` | Settings UI for viewing and rebinding shortcuts with capture mode |

### Custom events

Decoupled side effects via custom DOM events:
- `yapstack:toggle-chat` — Toggles the floating chat bar from anywhere
- `yapstack:confirm-delete-session` — Triggers delete confirmation dialog

### Store changes

- Settings version bumped from 5 → 9 (accumulated changes: v5→v6 `sidebarCollapsed`, v6→v7 `bufferMaxSeconds` + removed `backfillSeconds`, v7→v8 AI settings, v8→v9 `shortcutBindings`)
- Added `shortcutBindings: Record<string, string>` override map to settings

### Key decisions
- **In-app vs global split** — In-app shortcuts use `mod+key` format (capture phase). Global shortcuts use Tauri's `CmdOrCtrl+Key` format via the global-shortcut plugin.
- **`shortcutCaptureActive` shared flag** — Prevents shortcuts from firing while the user is rebinding in settings.
- **Custom events for decoupling** — `yapstack:toggle-chat` allows the keyboard shortcut hook to trigger FloatingChatBar without importing it or passing callbacks through the component tree.

### Dependencies added
- `@tauri-apps/plugin-global-shortcut`

---

## Phase 14 — AI Context Refactor + Hallucination Fixes

Two parallel improvements: modular AI context system enabling multi-session chat, and sidecar-level hallucination reduction.

### AI Context Refactor

| File | Purpose |
|------|---------|
| `lib/ai-context.ts` | `ContextSource`, `AIContextTools`, `SystemPromptBuilder`, `AIContextValue` types. Factory functions: `createSessionSources()`, `createSessionTools()`, `createSessionSystemPromptBuilder()`, `createMultiSessionSources()`, `createMultiSessionTools()`, `createMultiSessionSystemPromptBuilder()`. |
| `lib/ai-actions.ts` | `ActionDefinition` type, `ACTIONS` array (summarize, key-points, action-items, meeting-minutes), `getAction()`, `getActionIcon()` |
| `components/AIContextProvider.tsx` | React context provider wrapping `FloatingChatBar`. Manages source toggle state. `useAIContext()` hook. |
| `lib/db.ts` | `getChatMessages`/`deleteChatMessages` now take `contextKey` (not `sessionId`). `insertChatMessage` uses `context_key` field. New `getNotesForSessions()`. |
| `components/FloatingChatBar.tsx` | Consumes `AIContextValue` via `useAIContext()` instead of receiving individual props |

### Database changes

- **v8**: `chat_messages` — added `context_key TEXT NOT NULL`, made `session_id` nullable, added `idx_chat_messages_context(context_key, created_at)` index
- **v9**: `folders` — added `icon`, `color`, `description` columns. New `session_folders` junction table (many-to-many with unique constraint and indexes)

### Hallucination Fixes

| File | Change |
|------|--------|
| `yapstack-sidecar/src/main.rs` | Whisper params: `greedy(best_of=3)` (was 1), `no_context(true)`, `logprob_thold(-1.0)` (was -0.5), `max_tokens(100)` safety net. Added `normalize_for_repetition()` for punctuation-aware repetition detection. Expanded from ~6 to 47 filler hallucination patterns. Promoted filter logging to `info!`. Added Silero VAD integration via `WhisperVadParams`. |
| `crates/yapstack-transcription/src/model.rs` | VAD model management: `vad_model_path()`, `download_vad_model()`, `ensure_vad_model()`. Constants: `VAD_MODEL_FILENAME` (`ggml-silero-v6.2.0.bin`), `VAD_MODEL_URL`, `VAD_MODEL_SIZE_BYTES` (~885KB). |
| `crates/yapstack-transcription/src/whisper.rs` | `WhisperClient::spawn()` signature updated to accept `vad_model_path: Option<&Path>`. Passes `--vad-model` arg to sidecar when provided. |
| `commands/transcription.rs` | `init_whisper_client` now calls `ensure_vad_model()` before spawning, passes VAD model path to `WhisperClient::spawn()`. |

### Key decisions

- **Composable context via factory functions** — Session and multi-session contexts use the same `FloatingChatBar` component but different sources/tools/prompts assembled by factory functions. Adding a new context type only requires new factory functions in `ai-context.ts`.
- **`context_key` decouples chat from session_id** — Enables folder-level chat, "all sessions" chat, and "pinned" chat without schema gymnastics. Chat messages keyed by context string rather than FK.
- **Silero VAD as whisper.cpp preprocessing** — Different layer than the app-level `SourceVadState` in `live_transcription.rs`. App-level VAD chunks audio for Whisper. Silero VAD (inside whisper.cpp) skips non-speech within a chunk before decoding.
- **`no_context: true`** — Prevents whisper.cpp from carrying internal context between segments, which was a source of hallucination repetition across chunks.
- **Punctuation-normalized repetition detection** — `normalize_for_repetition()` inserts spaces around punctuation so "Yeah.Yeah.Yeah." is correctly detected as repetition.

---

## Phase 15 — General Settings + Voice Dictation

Added general settings tab, hold-to-talk voice dictation with dynamic slots, and the `YapStackIcon` component.

### General Settings

| File | Purpose |
|------|---------|
| `components/settings/GeneralTab.tsx` | Theme selection (light/dark/system), custom audio save location (folder picker + reveal), clear all sessions with confirmation |

Moved theme from inline settings into a dedicated General tab. Added `audioSaveLocation` to settings (v10 migration).

### Voice Dictation

| File | Purpose |
|------|---------|
| `commands/dictation.rs` | `clipboard_paste` Tauri command — clipboard write via `pbcopy`/`clip`, optional auto-paste via `osascript` keystroke simulation |
| `hooks/useDictation.ts` | Dictation lifecycle hook — state machine (idle → recording → transcribing → processing → done), controls bubble window, routes output |
| `components/DictationBubble.tsx` | Transparent always-on-top window showing dictation state (red=recording, blue=transcribing, purple=processing, green=done) |
| `components/settings/DictationTab.tsx` | Dynamic slot management — add/delete slots, configure name, keybind, output action, AI prompt per slot |
| `components/YapStackIcon.tsx` | SVG mask-based icon component using `bg-current` + CSS mask, used in DictationBubble |

### How dictation works

1. User presses and holds a global keybind → `useGlobalShortcuts` dispatches `yapstack:dictation-start` with `{ slotId }`
2. `useDictation` validates (enabled, engine ready, not in active recording), shows bubble, starts timing
3. User releases key → `yapstack:dictation-stop` dispatched
4. Hook captures audio via `triggerInstantCapture()`, transcribes via `transcribeAudio()`
5. If `slot.aiEnabled && slot.prompt`: post-processes transcription with AI API
6. Routes output based on `slot.outputAction`:
   - `"paste"`: clipboard + auto-paste via osascript
   - `"clipboard"`: clipboard only
   - `"new-note"`: creates manual session with notes, opens in main window

### App.tsx restructuring

Split into `MainApp` (main window with all hooks) and `App` (router). `App` checks `?window=dictation` query param — renders `DictationBubble` for the dictation window, `MainApp` for the main window. This avoids React rules-of-hooks violations from conditional hook calls.

### Dictation window configuration (tauri.conf.json)

```json
{
  "label": "dictation",
  "url": "index.html?window=dictation",
  "width": 220, "height": 64,
  "visible": false, "resizable": false, "decorations": false,
  "transparent": true, "alwaysOnTop": true, "skipTaskbar": true,
  "focus": false, "shadow": false
}
```

### Store changes

- Settings persist version: 9 → 12
- v9→v10: `audioSaveLocation: string | null`
- v10→v11: `dictation: DictationSettings` (enabled + slots array)
- v11→v12: Added `outputAction` to `DictationSlot` (default `"paste"`)
- Default: 1 slot ("Raw Dictation", paste output, no AI)

### Key decisions

- **Separate Tauri window for bubble** — Avoids DOM layering issues, enables always-on-top positioning outside the main window bounds, and transparent rendering without affecting main window layout.
- **Hold-to-talk (not toggle)** — Global shortcut Pressed/Released states map naturally to start/stop. More intuitive for dictation than toggle-on/toggle-off.
- **Dynamic slots (not fixed count)** — Users can add/delete slots with different keybinds, AI prompts, and output actions. No hardcoded slot limit.
- **`WebviewWindow.getByLabel()` is async** — Tauri v2 change from v1. Must `await` it in `useDictation`.
- **Keybinds in existing `shortcutBindings` map** — Dictation keybinds use keys `global.dictation-{slotId}` in the same override map as other shortcuts. No separate storage needed.
- **V1 limitation: unavailable during active recording** — WhisperClient is exclusively held by live transcription. Dictation requires the client for `transcribeAudio()`. Future: could use a second WhisperClient instance.
- **No new npm deps** — Reuses existing `@tauri-apps/plugin-global-shortcut` already added for keyboard shortcuts.

---

## Phase 16 — macOS Desktop UX Polish

Added system tray enhancements, close-to-minimize, floating recording indicator overlay, and build hardening.

### System Tray

| File | Change |
|------|--------|
| `src-tauri/src/lib.rs` | Rewrote `build_tray_menu(app, is_capturing, is_recording)` with dynamic items. Dedicated monochrome PNG tray icon via `Image::from_bytes(include_bytes!(...))` + `icon_as_template(true)`. Recording state tracked in 500ms polling loop. New event handlers: `open`, `new_session`, `new_session_bf_*`, `stop_session`. `show_main_window()` helper. |
| `src-tauri/icons/tray-icon.png` | 44×44 YapStack logo, black on transparent (macOS template icon) |
| `src-tauri/icons/tray-icon@2x.png` | 88×88 retina version |
| `src-tauri/Cargo.toml` | Added `image-png` feature to tauri for PNG icon loading |

Menu structure: Open YapStack → separator → Status (Idle/Listening/Recording) → Start/Stop Listening (disabled during recording) → separator → New Session + backfill submenu (30s, 1m, 2m, 5m, All) → [Stop Session] → separator → Quit YapStack.

### Close-to-Minimize

| File | Change |
|------|--------|
| `src/App.tsx` | `onCloseRequested` handler: `event.preventDefault()` → save position → `appWindow.hide()` |

Cmd+Q exits normally (OS-level, bypasses `closeRequested`). Tray "Quit" calls `app.exit(0)`.

### Recording Indicator

| File | Purpose |
|------|---------|
| `src/components/RecordingIndicator.tsx` | Transparent 56×120 overlay with pulsing YapStack icon (red ring) + `GripVertical` drag handle. Click icon → emits `recording-indicator:open-main`. |
| `src/hooks/useRecordingIndicator.ts` | Manages show/hide based on `activeSessionId` + `showRecordingIndicator` setting + main window focus. Positions at middle-right on first show. Listens for click events → navigates to active session + focuses main window. |
| `src-tauri/tauri.conf.json` | Added `recording-indicator` window (56×120, transparent, alwaysOnTop, no decorations/shadow, skipTaskbar, no focus) |

### Tray Events

| File | Purpose |
|------|---------|
| `src/hooks/useTrayEvents.ts` | Listens for `tray:new-session`, `tray:new-session-all`, `tray:stop-session` events. Guards on engine/capture state. Mounted in AppLayout. |

### Window Routing

`App.tsx` routes based on `?window=` URL param:
- `dictation` → `DictationBubble`
- `recording-indicator` → `RecordingIndicator`
- default → `MainApp`

### Settings

- Added `showRecordingIndicator: boolean` (default `true`) to Settings
- Persist version 12 → 13 with migration
- Toggle in `GeneralTab` ("Recording Indicator" / "Show floating indicator when recording and app is not focused")

### Build Hardening

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | `[profile.release]`: `strip = "symbols"`, `lto = "thin"`, `codegen-units = 1`, `panic = "abort"` |
| `vite.config.ts` | `sourcemap: false`, `esbuild.drop: ["console", "debugger"]` (production only), `legalComments: "none"` |
| `scripts/build-sidecar.sh` | `strip` binary after copy (belt-and-suspenders) |

### App Icons

Regenerated all icon sizes — white YapStack logo on zinc-900 (#18181b) background with 22% corner radius. Generated `.icns` via `iconutil` and `.ico` manually.

### Key Decisions

- **Dedicated tray icon PNG** — `app.default_window_icon()` with `icon_as_template(true)` renders as a white square because the app icon has no transparency. A separate `tray-icon.png` with alpha transparency is required for macOS template icons.
- **`image-png` feature** — Required on the `tauri` crate to parse PNG bytes via `Image::from_bytes()`.
- **Close-to-minimize** — Standard macOS desktop pattern. Users expect the tray to keep the app alive. Window position is saved to localStorage before hiding.
- **Cross-window events for recording indicator** — The indicator is a separate Tauri window (no shared Zustand store). Communication uses Tauri's `emit`/`listen` event system.
- **`CaptureStateDto` is an enum** — Comparison uses `matches!()` macro, not string equality.

---

## Phase 18 — Frontend Test Infrastructure

Added comprehensive frontend test coverage: 149 tests across 16 files covering library utilities, components, and the app root.

### Test infrastructure files

| File | Purpose |
|------|---------|
| `vitest.config.ts` | jsdom environment, `@/` path alias, setup file |
| `src/test/setup.ts` | `@testing-library/jest-dom` matchers + `ResizeObserver` polyfill |
| `src/test/tauri-mocks.ts` | Factory functions for all Tauri API mocks (core, event, window, dpi, webview, SQL, commands) |
| `src/test/match-media.ts` | `window.matchMedia` polyfill for jsdom |

### Test files created

**Library tests (7 files)**:
- `lib/utils.test.ts` — `cn()`, `formatShortcutDisplay()`, `formatTimestamp()`, `formatDuration()`
- `lib/shortcuts.test.ts` — Registry lookup, binding overrides, event conversion, shortcut categories
- `lib/folder-tree.test.ts` — Tree building, nesting, sorting, edge cases
- `lib/ai.test.ts` — Chat streaming, transcript context assembly, message building
- `lib/ai-prompts.test.ts` — System prompt construction, action directives, tool context
- `lib/ai-actions.test.ts` — Action registry, icons, action lookup
- `lib/ai-tools.test.ts` — Tool registration, execution, undo, schema validation

**Component tests (8 files)**:
- `components/AudioPlayer.test.tsx` — Play/pause, speed control, time display
- `components/EditableSegment.test.tsx` — Rendering, editing, context menu, confidence display
- `components/EmptyState.test.tsx` — Empty state messaging
- `components/FolderDialog.test.tsx` — Create/rename folder dialog
- `components/RecordingBeacon.test.tsx` — Pulsing indicator visibility
- `components/RecordingControls.test.tsx` — Recording controls state
- `components/SetupBanner.test.tsx` — Engine status banners
- `components/TrialExpiredOverlay.test.tsx` — Trial expiry overlay

**App test (1 file)**:
- `App.test.tsx` — Root routing by `?window=` param

### Root script updates

| Script | Command |
|--------|---------|
| `pnpm test` | `cargo test --all && pnpm --filter @yapstack/desktop test` |
| `pnpm test:frontend` | `pnpm --filter @yapstack/desktop test` |
| `pnpm test:rust` | `cargo test --all` |
| `pnpm test:watch` | `pnpm --filter @yapstack/desktop test:watch` |
| `pnpm lint` | `cargo fmt --all -- --check && cargo clippy --all -- -D warnings && pnpm --filter @yapstack/desktop lint` |
| `pnpm check` | Full CI gate: Rust build + test + fmt + clippy + TS typecheck + ESLint + vitest |

### Dev dependencies added
- `vitest` — Test runner
- `@testing-library/react` — React component testing utilities
- `@testing-library/user-event` — User interaction simulation
- `@testing-library/jest-dom` — Custom DOM matchers
- `jsdom` — Browser environment for tests

### Key decisions

- **Factory mock pattern** — Tauri mocks are factory functions (`tauriCoreMock()`, etc.) rather than side-effect imports. `vi.mock()` calls are hoisted above all imports by vitest, so the mock module must already be importable at hoist time. Factory functions return fresh mock objects per test file, avoiding shared state leaks.
- **Store injection via `useAppStore.setState()`** — Components read from Zustand; tests set state directly rather than mocking the store module. Simpler, tests real selector logic.
- **Module-level mocks for AI** — `ai.test.ts` and `ai-tools.test.ts` mock the OpenAI SDK and store at module level because these modules create clients at import time.
- **`ResizeObserver` polyfill** — jsdom doesn't implement `ResizeObserver`, which `react-resizable-panels` requires. A no-op polyfill in `setup.ts` prevents errors.

---

## Phase 19 — Prompt Context Decay

Added silence-triggered prompt decay to prevent Whisper hallucination after extended pauses. After 5+ seconds of silence across all sources, brief ambient noise (mousepad, cough, desk bump) caused Whisper to hallucinate words from the previous conversation because `shared_prompt` — the rolling 350-char context window — persisted indefinitely. Industry standard approach: clear prompt context after N seconds of all-source silence.

### What was built

| File | Change |
|------|--------|
| `commands/live_transcription.rs` | Added `prompt_decay_silence_seconds: Option<f32>` to `LiveTranscriptionConfig` (default 5.0, 0 to disable). Input validation. Extracted `check_prompt_decay()` helper function (testable without async loop). Two new state vars in live loop: `all_silent_since: Option<Instant>`, `prompt_seeded_from_backfill: bool`. Backfill seeding guard prevents re-seeding stale backfill data after decay clears the prompt. Decay check runs every 300ms after per-source VAD loop. 5 unit tests. |
| `stores/appStore.ts` | Added `promptDecaySilenceSeconds: number` to Settings (default 5). Persist version 14 → 15 with migration. Config construction passes `prompt_decay_silence_seconds` (null when 0 to disable). |
| `components/settings/TranscriptionTab.tsx` | New "Prompt Decay" `ButtonGroupSetting` in Advanced section: Off, 3s, 5s, 10s. |

### How it works

1. Every 300ms poll cycle, after per-source VAD processing, `check_prompt_decay()` checks if all sources are silent
2. If any source is speaking, `all_silent_since` resets to `None`
3. If all silent, `all_silent_since` records when silence began (via `get_or_insert_with`)
4. Once silence exceeds `prompt_decay_silence_seconds` and `shared_prompt` is non-empty, the prompt is cleared
5. When speech resumes, the prompt rebuilds naturally from new transcription via the existing `shared_prompt.push_str(&result.text)` in `transcribe_chunk`
6. Backfill seeding is one-shot: `prompt_seeded_from_backfill` flag prevents re-seeding from the bridged prompt after decay has cleared `shared_prompt`

### Key decisions

- **Silence-triggered reset (not gradual decay)** — Matches industry standard in WhisperX, whisper_streaming, and whisper.cpp application layers. Simpler and equally effective — Whisper's decoder either gets context or doesn't.
- **5s default** — Conservative enough to not clear context during normal speech pauses (typical 0.5-2s) but catches the problematic case of long silences followed by ambient noise.
- **Testable helper** — `check_prompt_decay()` is a pure function (modulo `Instant::now`) that can be unit tested without the async live transcription loop.
- **One-shot backfill seeding** — Without the `prompt_seeded_from_backfill` guard, a cleared prompt would be immediately re-filled from stale backfill context, defeating the purpose of decay.

---

## Phase 20 — Audio Pipeline Optimization

Expert review of the transcription pipeline identified suboptimal Whisper decoder parameters.

### What was built

| File | Change |
|------|--------|
| `crates/yapstack-sidecar/src/main.rs` | `best_of: 3` → `1` (deterministic at temp 0, all 3 candidates identical — wasted ~2x decoder time). `temperature_inc: 0.0` → `0.2` (enables whisper.cpp temperature fallback: retries at 0.2, 0.4… when initial decode produces high-entropy or low-logprob output). Fixed debug log from hardcoded values to actual dynamic `best_of`/`max_tokens`/`single_segment`. |
| `CLAUDE.md` | Updated whisper-rs params, removed overlap docs, added per-source accumulated_text and prompt decay details. |
| `docs/ARCHITECTURE.md` | Fixed resampler description (linear → sinc), updated Whisper params, removed overlap references, added per-source prompt details. |
| `docs/API_REFERENCE.md` | Fixed all stale Whisper param values (best_of, temperature_inc, no_speech_thold, single_segment, max_tokens), fixed resampler description. |

### Key decisions

- **best_of:1 + temperature_inc:0.2 are complementary** — Initial decode is still deterministic (temp 0), so `best_of:1` is correct. `temperature_inc:0.2` only kicks in as a fallback when whisper.cpp detects bad output (high entropy or low logprob). Temperature progression replaces `best_of` as the diversity mechanism but only fires when needed.
- **Documentation sweep** — Removed overlap references (never implemented), corrected "linear interpolation" to "sinc interpolation" across all docs, added per-source `accumulated_text` prompting details that were missing.

---

## Phase 21 — FE Refactor + Cross-Platform Overlay Windows + Tests

Frontend decomposition, macOS NSPanel integration for overlay windows, dead code cleanup, DB type improvements, and expanded test coverage.

### Cross-Platform Overlay Windows

| File | Change |
|------|--------|
| `apps/desktop/src-tauri/src/commands/mod.rs` | New `show_overlay_panel` and `hide_overlay_panel` Tauri commands. cfg-gated: macOS uses `tauri-nspanel` `ManagerExt::get_webview_panel()` on the main thread; non-macOS falls back to `WebviewWindow` show/hide. Also added `get_autostart_enabled` / `set_autostart_enabled` via `tauri-plugin-autostart`. |
| `apps/desktop/src-tauri/src/lib.rs` | `tauri-nspanel` plugin init (macOS only). `tauri_panel!` macro defines `OverlayPanel` type. On setup, dictation + recording-indicator windows converted to NSPanels with `PanelLevel::MainMenu`, non-activating style mask, stationary + `move_to_active_space` + `full_screen_auxiliary` collection behavior. |
| `apps/desktop/src-tauri/tauri.conf.json` | Added `visibleOnAllWorkspaces: true` to dictation and recording-indicator windows (non-macOS fallback). |
| `apps/desktop/src/hooks/useDictation.ts` | `showBubble()` / `hideBubble()` now use `commands.showOverlayPanel("dictation")` / `commands.hideOverlayPanel("dictation")` instead of direct window show/hide. |
| `apps/desktop/src/hooks/useRecordingIndicator.ts` | Show/hide uses `commands.showOverlayPanel("recording-indicator")` / `commands.hideOverlayPanel("recording-indicator")`. |

### FloatingChatBar Decomposition

| File | Purpose |
|------|---------|
| `hooks/useChatMessages.ts` | Extracted chat message lifecycle: send, stream, tool execution, undo (10s timeout), DB persistence. Returns `{ messages, isStreaming, undoState, handleSend, handleUndo, handleClearChat }`. |
| `components/chat/ChatInputBar.tsx` | Extracted input UI: textarea auto-resize, file attachments (500KB limit), actions popover, keyboard shortcuts. |
| `components/chat/ContextPill.tsx` | Toggleable context source pill for transcript/notes. |
| `components/chat/ModelPickerPill.tsx` | Popover-based model selector grouped by provider. |
| `components/FloatingChatBar.tsx` | Now a thin shell — delegates to `useChatMessages` + `ChatInputBar`. |

### Dead Code Removal

Deleted components: `EmptyState.tsx`, `SessionDetailView.tsx`, `SessionSidebar.tsx`.

Removed unused DB functions from `lib/db.ts`: `listPinnedSessions`, `updateSessionFolder`, `listSessionsByFolder`, `renameFolder`, `getSessionFolderIds`.

### DB Type Improvements

Added `SessionStatus` (`"recording" | "completed"`) and `SessionType` (`"manual" | "recording"`) string literal type aliases in `lib/db.ts`, replacing raw strings in `DbSession`.

### Test Coverage Expansion

6 new/expanded test files (21 total, 275 tests, up from 16 files / 149 tests):

| File | Tests | Notes |
|------|-------|-------|
| `components/AIChatMessage.test.tsx` | 15 | Tool badges, citation chips, markdown rendering |
| `components/NoteCard.test.tsx` | 14 | Card rendering, pin state, session types |
| `hooks/useClickOutside.test.ts` | 5 | Click outside detection hook |
| `lib/ai-context.test.ts` | 14 | Context factory functions, source assembly |
| `lib/analytics.test.ts` | 26 | Event tracking, fire-and-forget guarantees |
| `lib/utils.test.ts` | 39 | Expanded from 17 — added more edge cases |

New shared test utilities in `test/helpers.ts`: factory functions `makeSession()`, `makeSegment()`, `makeFolder()`, `makeChatMessage()` with sensible defaults.

### Key Decisions

- **NSPanel for overlay windows** — macOS NSPanels don't steal focus, appear above full-screen apps, and move with the active space. Standard `alwaysOnTop` windows can't achieve all three on macOS. The `visibleOnAllWorkspaces` tauri.conf.json property provides the non-macOS equivalent.
- **Platform-agnostic commands** — `show_overlay_panel` / `hide_overlay_panel` abstract the platform difference behind cfg-gating. Frontend code doesn't need to know about NSPanels.
- **Chat hook extraction** — `useChatMessages` encapsulates all streaming/tool/undo/DB logic so `FloatingChatBar` becomes a pure UI shell. Easier to test and reason about.
- **Test helpers** — Factory functions (`makeSession`, etc.) with sensible defaults reduce boilerplate across test files and ensure consistency.

---

## Phase 22 — Dictation History + AI Context Refactor

### Dictation History Persistence
- DB migration v10: `dictation_history` table with indexes (created_at, slot_id)
- Frontend CRUD: `insertDictationHistory`, `listDictationHistory`, `getDictationHistory`, `deleteDictationHistory`, `clearDictationHistory`, `updateDictationHistorySessionId` in `lib/db.ts`
- Store: `dictationHistory` state with `loadDictationHistory`, `deleteDictationHistoryEntry`, `clearDictationHistoryEntries` actions

### Dictation Activation Modes
- Settings v16: `activationMode: "hold" | "toggle"` added to `DictationSettings`
- Toggle state machine in `useGlobalShortcuts` via module-level `toggleActiveSlots: Set<string>`
- No-input detection (3s timer → yellow pulsing bubble with `"no-input"` state)
- `yapstack:dictation-idle` custom event for toggle mode cleanup

### Dictation History UI
- `DictationHistoryList`: entries grouped by day, "Clear All" button with confirmation
- `DictationHistoryCard`: badges (slot name, AI, output action), WAV playback, context menu (delete, move to note)
- `AppSidebar`: "Dictation" navigation button sets `ListFilter { type: "dictation" }`
- `NoteCardList` routes to `DictationHistoryList` when filter is dictation

### AI Context Refactor
- `ListChatContext` type and `resolveListContext()` factory centralizes context resolution for all `ListFilter` types
- `ListContextBar` component extracted from `AppLayout` (reduced ~90 lines of duplicated context setup)
- Dictation chat context: `createDictationSources`, `getDictationSystemPrompt`, `assembleDictationContext`
- Source ID disambiguation: `"notes"` → `"session-notes"` for multi-session contexts to avoid key collisions
- `AIContextProvider`: added `placeholder` prop for context-specific input hints
- `useChatMessages`: keying by `source.id` (not `source.type`) for correct context part assembly

### Analytics
3 new events: `dictation_history_cleared`, `dictation_history_entry_deleted`, `dictation_moved_to_note`

### Tests
- `ai-context.test.ts`: +5 tests for `resolveListContext` (all 4 context types + pluralization)
- Frontend total: 275 → 294 tests

---

## Phase 23 — Stream Health Monitoring & Auto-Restart

cpal audio streams can silently die (device disconnect, macOS sleep/wake, Bluetooth headphone switching) while `is_running` stays true. The only symptom is that `write_pos` freezes, causing the live transcription loop to produce no new segments.

### What was built

| File | Change |
|------|--------|
| `yapstack-audio/src/stream.rs` | `build_capture_stream()` accepts `stream_error: &Arc<AtomicBool>`. cpal error callback stores `true` with `Release` ordering. |
| `yapstack-audio/src/mic.rs` | Added `stream_error: Arc<AtomicBool>` field. `has_stream_error()` public API. Error flag reset before each `start()`. |
| `yapstack-audio/src/system/mod.rs` | Same pattern: `stream_error` field, `has_stream_error()`, reset on start. |
| `yapstack-audio/src/manager.rs` | `mic_has_stream_error()`, `system_has_stream_error()` delegate to captures. `restart_mic(device_name)`, `restart_system_audio()` stop old stream and start new one on the existing ring buffer. 4 new tests. |
| `commands/live_transcription.rs` | `StreamHealthEvent` DTO. `check_stream_health()` async function called each poll iteration. Constants: `STREAM_STALL_THRESHOLD_SECS = 2.0`, `STREAM_RESTART_MAX_ATTEMPTS = 3`. `SourceVadState` extended with `last_seen_write_pos`, `last_write_pos_advance`, `restart_attempts`. |
| `src/lib/events.ts` | `STREAM_HEALTH` event constant + `StreamHealthEvent` type. |
| `src/hooks/useLiveTranscriptionEvents.ts` | `stream-health` listener with toast notifications (success/error) and analytics tracking. |
| `src/lib/analytics.ts` | `trackStreamHealthEvent({ source, status })` — 1 new event. |

### How it works

1. **Layer 1 — cpal error flag** (instant): Each capture stream's cpal error callback sets an `Arc<AtomicBool>` flag. Checked via `mic_has_stream_error()` / `system_has_stream_error()`.
2. **Layer 2 — write_pos watchdog** (~2s latency): `SourceVadState` tracks `last_seen_write_pos` and `last_write_pos_advance`. If `write_pos` hasn't advanced for 2 seconds, the stream is considered stalled.
3. **Restart**: `AudioManager::restart_mic()` / `restart_system_audio()` stop the old (dead) stream and start a new one on the same ring buffer — no audio data is lost from the buffer. Up to 3 attempts per source; on success, `restart_attempts` resets to 0.
4. **Frontend**: Toast notifications for restart success/failure. `stream_health_event` analytics with source and status.

### Key decisions

- **Two detection layers** — cpal error callbacks catch some failures instantly, but not all (some platforms silently stop producing samples). The write_pos watchdog catches the rest with a 2s delay.
- **Buffer preservation** — Restart creates a new cpal stream writing to the same ring buffer. The monotonic `write_pos` continues from where it was. No audio data is lost and the live transcription loop resumes seamlessly.
- **3 restart attempts** — Prevents infinite restart loops if the device is truly gone. After 3 failures, the source is abandoned and the user sees a persistent error toast.
- **Default device on restart** — The restart uses the default device (not the original device name) because the original device may have been disconnected.

---

## What's Not Yet Built

- **End-to-end integration tests** — capture audio, transcribe, verify text (unit + component tests now exist; integration tests still needed)
- **CI/CD pipeline** — GitHub Actions for cross-platform builds, sidecar compilation
- **Temp file cleanup** — WAV files from capture accumulate; needs cleanup after transcription
- **Sidecar auto-restart** — `WhisperClient` detects crashes but doesn't restart automatically
- **Progress events during sidecar inference** — the sidecar emits `Progress` responses and the client handles them (skipping to wait for final result), but whisper-rs doesn't provide a progress callback during inference itself
- **Multi-turn tool calling** — Current AI tool flow is single-turn (execute tools, don't send results back). Multi-turn would allow the AI to observe tool results and take follow-up actions.
- **Additional AI tools** — `suggest_folder`, `create_tag`, `export_pdf`, `search_sessions` are natural extensions of the tool registry
- **Sharing** — `shares` table exists but no sharing UI or backend logic
- **Dictation during active recording** — WhisperClient is exclusively held by live transcription; dictation is unavailable while recording. Requires a second WhisperClient instance or queuing mechanism.
- **Multi-platform dictation testing** — Auto-paste (`osascript`) is macOS-only. Windows implementation needs testing. Linux not yet supported.
