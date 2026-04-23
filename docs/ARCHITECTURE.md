# Architecture

## Overview

YapStack is a desktop app for real-time audio capture and transcription. Built with Tauri v2 (Rust backend) + React 19 (TypeScript frontend). Audio is captured via `cpal`, stored in lock-free ring buffers, exported to WAV, and transcribed by a sidecar process that supports two engines as first-class peers: **Whisper** (whisper-rs, 99 languages) and **NVIDIA Parakeet TDT v3** (parakeet-rs + ONNX Runtime, 25 European languages, optional Sortformer speaker diarization). The user picks the engine in Settings; the sidecar is spawned with `--engine whisper|parakeet` and dispatches IPC requests through a `TranscriptionBackend` trait so adding a third engine is one feature flag plus one impl. Sessions and segments are persisted to SQLite via `tauri-plugin-sql`.

## Workspace Layout

```
yapstack/
├── Cargo.toml                  # Workspace root
├── crates/
│   ├── yapstack-common/           # Shared types, config, IPC protocol
│   ├── yapstack-audio/            # Audio capture, ring buffers, mixing, WAV export
│   ├── yapstack-transcription/    # Model management, sidecar client
│   └── yapstack-sidecar/          # Standalone binary for Whisper transcription
├── apps/
│   └── desktop/
│       ├── src/                # React frontend
│       └── src-tauri/          # Tauri backend (commands, state management)
└── scripts/
    ├── build-sidecar.sh        # Build sidecar for target triples
    └── download-test-fixtures.sh
```

## Crate Dependency Graph

```
yapstack-desktop
├── yapstack-audio
│   └── yapstack-common
├── yapstack-transcription
│   └── yapstack-common
└── yapstack-common

yapstack-sidecar (standalone binary)
└── yapstack-common
```

`yapstack-sidecar` is a separate binary, not linked into the desktop app. It communicates via JSON-line IPC over stdin/stdout.

## Module Responsibilities

### yapstack-common
Shared types used across all crates. No business logic.
- `audio.rs` — `deinterleave_to_mono()`, `resample()` (rubato sinc, no-op when from_rate == to_rate). Used by `yapstack-audio` internally, `live_transcription.rs` directly, and the Parakeet/Sortformer backends in `yapstack-sidecar`.
- `config.rs` — `AppConfig`, `AudioConfig` (only `capture_history_seconds`), `TranscriptionConfig` with serde
- `types.rs` — Domain types (`CaptureState`, `CaptureSource`, `PermissionStatus`, `EngineKind`, etc.). `TranscriptSegment` carries an optional `speaker_id` for Parakeet+Sortformer output. IPC protocol (`SidecarRequest`, `SidecarResponse`) is engine-agnostic; `Transcribe` carries an optional `diarization: bool`.
- `engines.rs` — Static `engine_catalogue()` exposing `EngineDescriptor { kind, display_name, languages, supports_diarization, supports_initial_prompt }` per engine. Single source of truth for engine capabilities + supported languages, consumed by both backend validation and the frontend cascading dropdowns (via the `get_engine_catalogue` Tauri command).

### yapstack-audio
All audio capture and processing. Core of the application.
- `ring_buffer.rs` — Lock-free SPSC ring buffer for real-time audio. `UnsafeCell` + atomics. Producer API is zero-alloc (safe for audio callbacks). Consumer API allocates snapshots.
- `device.rs` — Device enumeration via `cpal`
- `mic.rs` — Microphone capture stream management. `stream_error: Arc<AtomicBool>` for cpal error propagation.
- `system/` — System audio capture (macOS via cpal loopback on default output device, Windows stub, fallback `Unavailable`)
- `manager.rs` — `AudioManager` orchestrates mic + system capture, session tracking, instant capture. Stream restart (`restart_mic`, `restart_system_audio`) preserving ring buffers.
- `mixer.rs` — Pure functions for mixing mic + system audio with gain/normalization
- `export.rs` — WAV file export via `hound` (16-bit PCM, f32 clamping). `SessionWavWriter` for incremental streaming WAV during live sessions.
- `capture.rs` — Data types: `CapturedAudio`, `SessionMark`, `CaptureResult`, `BufferPositions`, `SeparateExtraction`
- `error.rs` — `AudioError` enum with `From` impls for cpal and hound errors

### yapstack-transcription
Model management and sidecar client. No whisper-rs / parakeet-rs dependencies — those live entirely in the sidecar.
- `model.rs` — `ModelManager` manages three model families with HuggingFace download + streaming SHA-256 verification:
  - **Whisper** (single ggml file): `ModelSize { Tiny, Base, Small, Medium }`, `download`, `verify_checksum`, `delete`, `list_all`.
  - **Whisper VAD** (Silero, ~885KB, auto-downloaded): `vad_model_path`, `download_vad_model`, `ensure_vad_model`.
  - **Parakeet** (multi-file ONNX bundle in `models_dir/parakeet-<variant>/`): `ParakeetVariant::TdtV3`, `parakeet_model_dir`, `parakeet_is_available` (checks all required files), `download_parakeet` (loops over the variant's `files()` list with per-file SHA verify), `delete_parakeet`, `ensure_parakeet`.
  - **Sortformer** (single ONNX file, ~50MB): `SortformerVariant::V2_1`, `sortformer_model_path`, `download_sortformer`, `ensure_sortformer`.
- `client.rs` (was `whisper.rs`) — `TranscriptionClient` spawns the sidecar process and communicates via JSON-line IPC. `spawn()` takes `engine: EngineKind`, `model_path`, `vad_model_path?` (Whisper-only), `sortformer_model_path?` (Parakeet-only), and `coreml_cache_dir?` (Parakeet-only). `transcribe_with(audio, language, prompt, diarization)` exposes the per-call diarization flag. `respawn()` re-spawns the sidecar preserving engine + all paths + `next_id`. A `WhisperClient` type alias is retained for transitional compat with the Tauri layer.
- `error.rs` — `TranscriptionError` with variants for download, sidecar, timeout, IO, HTTP

### yapstack-sidecar
Standalone binary. Reads JSON-line requests from stdin, writes responses to stdout. Logging goes to stderr.

**CLI**: `yapstack-sidecar [--engine whisper|parakeet] [--model PATH] [--vad-model PATH] [--sortformer-model PATH] [--coreml-cache-dir PATH]`

**Architecture**: `engines/mod.rs` defines a `TranscriptionBackend` trait + shared text-cleanup helpers. Concrete impls in `engines/whisper.rs` (whisper-rs + flash attention + Silero VAD) and `engines/parakeet.rs` (ParakeetTDT + Sortformer post-pass + ORT EP selection). `main.rs` is a thin IPC dispatcher that picks one backend at startup and forwards every request through the trait. When the engine the sidecar was spawned with isn't compiled in (e.g. `--engine parakeet` on a `--features whisper` build), every IPC request returns `engine 'X' not compiled in this build`.

**Whisper backend (`engines/whisper.rs`)**: Loads whisper-rs model with flash attention, converts audio to 16kHz mono (stereo→mono averaging + sinc interpolation resampling via rubato), transcribes WAV files with optional `initial_prompt` for context continuity, filters hallucination segments via `should_include_segment()`.
- Whisper params: `greedy(best_of=1)`, `no_context(true)`, `logprob_thold(-1.0)`, `max_tokens(100/200)`, `suppress_blank(true)`, `suppress_nst(true)`, `temperature_inc(0.2)`, `no_speech_thold(0.45)`, `single_segment(adaptive)`
- **Silero VAD**: Optional `--vad-model` arg enables voice activity detection as whisper.cpp preprocessing (threshold 0.5, min_speech 250ms, min_silence 100ms, pad 30ms). Skips non-speech audio before decoding, reducing hallucinations.
- Hallucination filtering: drops empty text, special tokens (`[BLANK_AUDIO]`, `[MUSIC]`), low-confidence segments (< 0.4), excessive word/phrase repetition (3+ consecutive identical words with punctuation-normalized detection via `normalize_for_repetition()`), and 47 known filler/hallucination patterns ("thank you", "thanks for watching", "yeah", "um", "so", etc.) at marginal confidence (< 0.6).

**Parakeet backend (`engines/parakeet.rs`)**: Loads `ParakeetTDT::from_pretrained(model_dir, exec_config)` from a directory containing `encoder-model.onnx` + `encoder-model.onnx.data` + `decoder_joint-model.onnx` + `vocab.txt`. Resamples audio to 16 kHz mono via `yapstack_common::audio::resample` before calling `transcribe_samples` (parakeet-rs 0.3.5 rejects other rates despite its docs). Token-level timestamps grouped into segments at 0.5 s silence gaps + 12 s soft cap. Same `should_include_segment` post-filter as Whisper (mostly a no-op for Parakeet which doesn't hallucinate the same patterns).
- **ORT execution provider** selected at startup via `YAPSTACK_PARAKEET_ACCEL=auto|cpu|coreml|webgpu` env var. `auto` uses CoreML when compiled in *and* the model dir contains no external `.onnx.data` files (TDT v3 *does* — so Auto falls through to CPU on this model). Per-load CPU fallback at the backend level catches CoreML/WebGPU load failures and degrades silently — the sidecar never returns "no model loaded".
- **Sortformer post-pass** (when `Transcribe.diarization=true` and the sidecar was spawned with `--sortformer-model`): runs Sortformer on the same resampled 16 kHz audio, returns `Vec<SpeakerSegment>` with sample-range + `speaker_id`. `assign_speakers()` maps speaker ranges onto produced transcript segments by maximum-overlap (samples → ms → segment range overlap) and populates `TranscriptSegment.speaker_id`. Sortformer model is loaded lazily on first diarization request to avoid the cost when diarization is off.

### yapstack-desktop (Tauri app)
Tauri commands layer. Thin wrappers that convert between domain types and DTOs (with `specta::Type` for TypeScript generation).
- `commands/error.rs` — `CommandError` tagged union with 6 error kinds (`Audio`, `Transcription`, `NotInitialized`, `InvalidInput`, `NotFound`, `Internal`). All Tauri commands return `Result<_, CommandError>`. `From` impls for `AudioError`, `TranscriptionError`, `std::io::Error`.
- `commands/audio.rs` — Device listing, capture start/stop, buffer snapshots, instant capture, session start/end. `MixConfigDto::sanitized()` validates gain values at the command boundary.
- `commands/transcription.rs` — Model management, transcription, sidecar lifecycle. Locks released before async I/O.
- `commands/live_transcription.rs` — Real-time transcription controller with per-source VAD (`SourceVadState`), concurrent backfill processing, prompt context windowing, silence trimming, stream health monitoring with auto-restart. Shared `TranscriptionContext` struct for immutable config passing. Extracts `WhisperClient` from shared state for zero-contention private use during the loop. Streams session WAV incrementally via `SessionWavWriter`.
- `commands/dictation.rs` — `clipboard_paste` command for voice dictation output. Writes text to system clipboard via `pbcopy` (macOS) or `clip` (Windows), optionally triggers auto-paste via `osascript` keystroke simulation.
- `commands/mod.rs` — `show_overlay_panel`, `hide_overlay_panel` commands (cfg-gated: macOS uses NSPanel via `tauri-nspanel`, non-macOS falls back to WebviewWindow show/hide). `get_autostart_enabled`, `set_autostart_enabled` commands via `tauri-plugin-autostart`.

## Data Flow

### Audio Capture
```
Microphone/System Audio
    → cpal callback (real-time thread, zero-alloc)
    → AudioRingBuffer.write() (lock-free, Release ordering)
    → AudioRingBuffer.snapshot() (app thread, Acquire ordering)
    → Vec<f32> samples
```

### Instant Capture
```
Frontend: trigger_instant_capture(seconds, source, mix_config)
    → AudioManager.extract_captured_audio() → snapshot from ring buffers
    → deinterleave_to_mono() per buffer (stereo → mono)
    → mixer::mix_to_mono() (if Mixed source, requires matching sample rates)
    → export::write_wav_to_temp() → mono WAV file at device sample rate
    → CaptureResult { file_path, duration, ... }
```

### Session Capture
```
Primary path (live transcription — used by the app for all recordings):
    start_live_transcription(config with session_id)
        → Creates SessionWavWriter at $APP_DATA_DIR/audio/{session_id}.wav
        → Every 300ms: extract_since() from ring buffer → append to WAV file
        → On stop: final flush → finalize WAV header → emit "session-wav-ready"
    No audio lost regardless of session length (ring buffer only holds last 300s,
    but the WAV file captures everything incrementally).

Fallback path (short sessions or re-export):
    start_session() → records write_pos for mic + system buffers
        ... audio accumulates in ring buffers ...
    end_session() → snapshot_since(start_pos) → deinterleave_to_mono() → mix → mono WAV → CaptureResult
    Limited to ring buffer capacity. Kept for export_session_wav re-export.
```

### Transcription
```
Frontend: init_transcription_client(engine, whisper_model?, parakeet_variant?, enable_diarization)
                                    (or legacy init_whisper_client(size) — shim that calls the engine-aware path with EngineKind::Whisper)
    → Validates the engine's selected model is on disk; returns NotFound if not
    → For Parakeet+diarization, also ensure_sortformer() (auto-downloads ~50MB Sortformer ONNX)
    → For Whisper, ensure_vad_model() (auto-downloads ~885KB Silero ONNX)
    → Resolves CoreML cache dir under $APP_DATA_DIR/cache/coreml/ (Parakeet only)
    → spawns yapstack-sidecar with: --engine, --model, [--vad-model | --sortformer-model + --coreml-cache-dir]
    → JSON-line IPC over stdin/stdout

Frontend: transcribe_audio(wav_path)
    → TranscriptionClient.transcribe() / .transcribe_with(diarization=true)
    → sends {"type":"transcribe", ..., diarization} to sidecar stdin
    → sidecar dispatches via TranscriptionBackend trait:

      Whisper backend:
        → loads WAV (any sample rate / channel count)
        → converts to mono (averaging channels) + resamples to 16kHz (sinc via rubato) if needed
        → runs whisper inference with optional initial_prompt
        → segments[*].speaker_id = None

      Parakeet backend:
        → loads WAV at native rate
        → resamples to 16kHz mono via yapstack_common::audio::resample
        → calls ParakeetTDT.transcribe_samples (CoreML/WebGPU/CPU per env var)
        → groups word-level tokens into segments at silence gaps
        → if diarization=true: also runs Sortformer on the same audio,
          assigns speaker_id by max-overlap onto produced segments
        → drops opts.initial_prompt silently (TDT decoder has no prompt input)

    → returns {"type":"transcription", text, segments[* speaker_id], duration_ms}
    → TranscriptionResult { text, segments, duration_ms }
```

### Live Transcription
```
Frontend: start_live_transcription(config)
    → Extracts WhisperClient from shared state → private Arc<Mutex<Option<WhisperClient>>>
      (zero contention — other commands get "not available" instead of blocking)
    → Creates TranscriptionContext (immutable: whisper_client, shared_whisper_state, app_handle, config, start time)
    → If config.session_id set: creates SessionWavWriter at $APP_DATA_DIR/audio/{session_id}.wav
    → Spawns async task with two concurrent tracks:

    Track 1 — Backfill (if backfill_seconds > 0):
        1. Rewind cursors by backfill_seconds from current write_pos
        2. Extract historical audio per source
        3. Chunk by max_chunk_seconds, trim_leading_silence() each chunk
        4. Skip entirely silent chunks
        5. Transcribe interleaved (window 0 all sources, window 1, etc.)
        6. Emit "live-segment" events with is_backfill=true
        7. Emit "backfill-complete" event

    Track 2 — Live VAD loop (runs concurrently; per-call diarization gated on
              ctx.config.diarization, initial_prompt gated on
              client.engine() == EngineKind::Whisper):
        1. Poll every tuning.poll_interval (Whisper 300ms; Parakeet 100ms)
        2. Single lock: per-source extract_source_audio() for Silero +
           extract_since() for WAV flush. Raw mono samples, not RMS.
        3. Outside the lock:
            a. Write WAV samples (disk I/O doesn't hold AudioManager)
            b. Feed each source's samples through the shared Silero V5
               session via score_stream() — returns one probability per
               32 ms frame, maintained across polls by per-source LSTM
               state in SileroSource
        4. poll_vad() per source, once per Silero frame — state machine
           iterates every probability so intra-poll speech isn't missed.
           Best-action summary (ForceChunk > Chunk > None) picked per tick.
        5. On VadAction::Chunk (silence detected) or ForceChunk (max duration):
            a. extract_sources_since()
            b. write_wav_to_temp() → temp WAV
            c. WhisperClient.transcribe() with prompt context (last N chars)
            d. Emit "live-segment" event with segments + metadata
        6. On stop: force-chunk any speaking source, then exit loop

    → After loop exits (cleanup):
        1. Final WAV flush + finalize (if session_id was set)
        2. Emit "session-wav-ready" event with { session_id, file_path, duration_seconds }
        3. Returns WhisperClient to shared state (even after panic via AssertUnwindSafe)
    → Per-source VAD: SourceVadState tracks mic/system independently
    → Prompt context: per-source accumulated_text (up to 1000 chars) truncated to prompt_context_chars (default 350) for Whisper initial_prompt
    → Prompt decay: clears both shared_prompt and per-source accumulated_text after prompt_decay_silence_seconds (default 5s) of
      all-source silence. Prevents stale context from causing hallucination after long pauses.
      Backfill seeding is one-shot (prompt_seeded_from_backfill guard prevents re-seeding after decay).
    → Stopped via oneshot channel from stop_live_transcription()
```

### Stream Health Monitoring

The live transcription loop includes a stream health watchdog (`check_stream_health()`) that runs every 300ms poll iteration to detect and recover from silently-dead cpal streams (e.g., device disconnect, sleep/wake).

**Two detection layers**:
1. **cpal error callback flag** (instant) — Each capture stream stores an `Arc<AtomicBool>` (`stream_error`) that the cpal error callback sets to `true` on any stream error. Checked via `mic_has_stream_error()` / `system_has_stream_error()`.
2. **`write_pos` stall watchdog** (~2s latency) — `SourceVadState` tracks `last_seen_write_pos` and `last_write_pos_advance`. If `write_pos` hasn't advanced for `STREAM_STALL_THRESHOLD_SECS` (2.0s), the stream is considered stalled.

**Watchdog fields on `SourceVadState`**:
- `last_seen_write_pos: usize` — last observed buffer write position
- `last_write_pos_advance: Instant` — when write_pos last changed
- `restart_attempts: u32` — restart counter per source

**Restart behavior**: Up to `STREAM_RESTART_MAX_ATTEMPTS` (3) restart attempts per source. `AudioManager::restart_mic()` / `restart_system_audio()` stop the old stream and start a new one on the same ring buffer — no audio data is lost from the buffer. On successful restart, `restart_attempts` resets to 0.

**Events**: Emits `"stream-health"` Tauri events with `StreamHealthEvent { source: AudioSourceLabel, status: String, message: String }`. Status values: `"restarted"` (success), `"restart_failed"` (will retry), `"restart_abandoned"` (max attempts reached). Frontend shows toast notifications and tracks `stream_health_event` analytics.

## State Management

### Backend (Rust)
Four `Arc<Mutex<T>>` states managed by Tauri:
- `AudioManagerState` — audio capture lifecycle, ring buffers, sessions
- `ModelManagerState` — model download/delete/list (Whisper + Parakeet + Sortformer)
- `WhisperClientState` — `Option<TranscriptionClient>`, sidecar process lifecycle (alias name kept for backward compat; the wrapped value is the engine-agnostic `TranscriptionClient`, regardless of which engine was selected)
- `LiveTranscriptionState` — `Option<LiveTranscriptionController>`, live transcription task + stop signal

Startup also runs `db::ensure_runtime_schema()` *before* tauri-plugin-sql wires up — this opens the SQLite DB directly via `rusqlite` and adds `segments.speaker_id INTEGER` if missing. The column is intentionally outside the migration list because some local dev DBs have a "ghost" v11 entry from another branch in `_sqlx_migrations`, which makes sqlx silently refuse any v12+ migration. The startup hook sidesteps the entire ordering problem; idempotent on every boot.

### Frontend (TypeScript)
- **State**: Zustand store (`stores/appStore.ts`) with persisted settings (**version 22**, migrations for schema changes). Settings include capture source, **selected engine** (`"Whisper" | "Parakeet"`, peers — Whisper is the upgrade-safe default), Whisper model size, **selected Parakeet variant**, **diarization enabled**, **per-session `speakerNames` map** (renames `Speaker N` → custom labels, persisted client-side), language, VAD params, prompt context, prompt decay silence, theme, sidebar state, buffer size, AI settings, shortcut bindings, audio save location, dictation settings, `showRecordingIndicator`. The store also caches the engine catalogue (`engineCatalogue: EngineDescriptorDto[]`) and Parakeet/Sortformer download status (`parakeetModels`, `sortformerStatus`) loaded on `autoSetup`.
- **Persistence**: SQLite via `tauri-plugin-sql` (`lib/db.ts`) for sessions, segments, notes, note versions, folders, session_folders (many-to-many), chat messages, and dictation history. DB file at app data dir. 10 DB migration versions; `segments.speaker_id INTEGER` is added by `db::ensure_runtime_schema()` at app startup (outside the migration list — see [API Reference](API_REFERENCE.md) and the comment in `db.rs`).
- **Type generation**: Specta-generated types in `src/lib/types.ts` (auto-generated, excluded from tsconfig). Tauri command wrappers in `src/lib/tauri.ts`.
- **Serialization queue**: `onLiveSegment` uses a promise queue (`segmentQueueTail`) to prevent concurrent backfill + live events from racing on DB writes.
- **Hooks**: `useAutoSetup` (engine init), `useCaptureEvents` (backend status push), `useLiveTranscriptionEvents` (segment/phase/backfill/session-wav-ready events), `useCreateSession` (session creation guard), `useDownloadProgress` (model download), `useKeyboardShortcuts` (in-app capture-phase keydown in AppLayout), `useGlobalShortcuts` (Tauri global-shortcut plugin in App.tsx), `useDictation` (hold-to-talk or toggle mode voice dictation lifecycle in App.tsx), `useRecordingIndicator` (show/hide floating overlay when recording + unfocused, in App.tsx), `useTrayEvents` (listen for tray menu actions, in AppLayout), `useChatMessages` (chat message lifecycle: send, stream, tool execution, undo, DB persistence).
- **Navigation**: Three views: `"note-list"` | `"note-detail"` | `"settings"`. `ListFilter` supports `{ type: "all" | "pinned" | "folder" | "dictation", folderId?: string }`.
- **Views**: `AppLayout` → `AppSidebar` + main content (`NoteCardList` | `NoteDetailView` | `SettingsPanel`). `NoteDetailView` handles multiple layouts:
  - **Active recording**: `SessionHeaderV2` + `ChatView` (transcript bubbles with backfill indicator) + `RecordingControls`
  - **Completed transcription**: `SessionHeaderV2` + `AudioPlayer` + resizable split pane (`ChatView` left, `NoteEditor` + `NoteHistoryPanel` right)
  - **Manual notes**: `SessionHeaderV2` + full-width `NoteEditor` + `NoteHistoryPanel`
- **Settings UI**: Tabbed (`AudioTab`, `TranscriptionTab`, `GeneralTab`, `ShortcutsTab`, `DictationTab`). `TranscriptionTab` renders an **engine → model → language cascade**: engine radio (Whisper/Parakeet, peers), conditional model picker (`ModelSection` for Whisper sizes, new `ParakeetModelSection` for Parakeet variants), language dropdown derived from `engineCatalogue` (clamps to "auto" when current language isn't in the new engine's supported set), Switch-style **diarization toggle** (greyed out unless engine supports it; first enable lazily downloads Sortformer + re-inits the client). `GeneralTab` manages theme, audio save location, and session clearing. `DictationTab` manages dictation enable/disable, dynamic slot configuration (name, keybind, output action, AI prompt), and slot add/delete.
- **Transcript view (`TranscriptSegments.tsx`)**: Wrapper around `EditableSegment` rendered by `ChatView`. Falls back to a flat segment list when no segment carries a `speaker_id` (Whisper sessions render unchanged). Otherwise groups consecutive same-speaker segments under a `Speaker N` header with a 4-color palette. Speaker headers are inline-editable; renames persist to per-session `speakerNames` Zustand map.
- **AI prompts**: `ai-prompts.ts::getSystemPrompt(directive, ..., { hasSpeakers })` injects a `SPEAKER_INSTRUCTION` paragraph telling the model to attribute statements correctly when the active session has diarization data. `ai.ts::assembleTranscriptContext(segments, speakerNames?)` adds `(SpeakerName)` prefixes per segment.
- **Global features**: `SearchCommand` (Cmd+K fuzzy search via `cmdk`), drag-and-drop sessions into folders via `@dnd-kit`.
- **AI Chat**: `AIContextProvider` wraps `FloatingChatBar` to provide context-dependent AI chat. `FloatingChatBar` delegates chat logic to the `useChatMessages` hook and input UI to `ChatInputBar` (with `ContextPill` and `ModelPickerPill` sub-components in `components/chat/`). Factory functions in `ai-context.ts` assemble sources/tools/prompts for session vs. multi-session (folder) contexts. `resolveListContext()` centralizes context resolution for non-session views (all, pinned, folder, dictation). `ListContextBar` renders the floating chat bar for list-level contexts. See [AI Chat & Tool Calling](#ai-chat--tool-calling) below.
- **Dictation**: `DictationBubble` renders in a separate Tauri window. `YapStackIcon` provides a reusable SVG mask-based icon component. See [Dictation](#dictation) below.
- **Recording Indicator**: `RecordingIndicator` renders in a separate transparent always-on-top window (56×120). Shows a pulsing YapStack icon with drag handle when recording and main window is unfocused. Clicking the icon opens the main window to the active session. Controlled by `useRecordingIndicator` hook (uses `commands.showOverlayPanel("recording-indicator")` / `hideOverlayPanel`). Toggleable via `showRecordingIndicator` setting.
- **System Tray**: Enhanced tray menu with dynamic items based on capture/recording state. Menu: Open YapStack → Status (Idle/Listening/Recording) → Start/Stop Listening (disabled during recording) → New Session + backfill submenu → Stop Session → Quit. Tray icon uses a dedicated monochrome PNG template (`tray-icon.png`). Events dispatched via Tauri `emit`/`listen` to `useTrayEvents` hook.
- **Close-to-minimize**: Main window `closeRequested` event is intercepted — window hides instead of closing. Cmd+Q still exits (OS-level). Tray "Quit" calls `app.exit(0)`.
- **Cross-platform overlay windows**: Dictation and recording-indicator windows are converted to NSPanels on macOS via `tauri-nspanel` (`#[cfg(target_os = "macos")]`). NSPanel config: `PanelLevel::MainMenu`, non-activating (`StyleMask::nonactivating_panel`), stationary + `move_to_active_space` + `full_screen_auxiliary` collection behavior. On non-macOS: `visibleOnAllWorkspaces: true` in `tauri.conf.json` + standard WebviewWindow show/hide. Frontend uses `commands.showOverlayPanel(label)` / `commands.hideOverlayPanel(label)` — platform-agnostic.

## AI Chat & Tool Calling

### Overview

The AI chat system uses OpenAI-compatible APIs (OpenAI, OpenRouter, or custom endpoints) with **function/tool calling** to let the AI directly mutate session data. This creates a two-way bridge: the AI reads transcript + notes context, and writes back title changes, note updates, and pin state.

The system is designed around two self-contained registries — **actions** (prompt directives) and **tools** (mutations with undo) — so that extending the AI requires minimal file changes.

### Architecture

```
User triggers action (quick action button) or freeform message
    → FloatingChatBar.handleSend(actionDef?: ActionDefinition)
    → Resolve directive: actionDef.directive ?? GENERAL_DIRECTIVE
    → Assemble context from enabled sources (transcript, notes)
    → buildSystemPrompt(directive, contextParts, attachments)
        → getSystemPromptWithToolContext(directive, transcript, notes, attachments, sessionMeta)
    → streamChatWithTools(client, model, messages, tools)
        → OpenAI streaming API with tool schemas
        → yields: token events (streamed text) + tool_call events (accumulated)
    → For each tool_call: executeTool(name, args, toolContext)
        → Mutates DB (title, notes, pin)
        → Returns ExecutedTool with undoData
    → Prepend [tool:name] badge lines to message content
    → Persist to chat_messages DB
    → Post-tool refresh via onToolsExecuted → getToolEffects() → refresh by effect category
    → Show toast with 10s undo window
```

### File Structure

| File | Responsibility |
|------|---------------|
| `lib/ai-actions.ts` | Self-contained action definitions — each carries its own `directive` string |
| `lib/ai-tools.ts` | Self-contained tool registry — schema + executor + undo + `affects` metadata per tool |
| `lib/ai-prompts.ts` | Prompt assembly: `GENERAL_DIRECTIVE`, `getSystemPrompt(directive, ...)`, citation/notes guidance |
| `lib/ai-context.ts` | Context provider types, factory functions for session/multi-session context assembly |
| `lib/ai.ts` | OpenAI client, streaming (with and without tools), context assembly, markdown→HTML via `marked` |
| `components/AIContextProvider.tsx` | React context provider, source toggle state management |
| `components/FloatingChatBar.tsx` | Chat UI shell, delegates chat logic to `useChatMessages` hook and input to `ChatInputBar` |
| `components/chat/ChatInputBar.tsx` | Input textarea, file attachments, action buttons, context/model pills |
| `components/chat/ContextPill.tsx` | Toggleable context source pill (transcript, notes) |
| `components/chat/ModelPickerPill.tsx` | Popover-based model selector grouped by provider |
| `hooks/useChatMessages.ts` | Chat message lifecycle: send, stream, tool execution, undo, DB persistence |
| `components/AIChatMessage.tsx` | Message rendering with tool badges and citation chips |

### Action Definitions (`ai-actions.ts`)

Actions are self-contained `ActionDefinition` objects. Each carries its own system prompt directive — no external lookup table needed.

```typescript
interface ActionDefinition {
  id: string;              // e.g. "summarize", "key-points", or a custom ID
  label: string;           // Display label in the action menu
  description: string;     // Tooltip / secondary text
  icon: LucideIcon;        // Icon component
  requiresTranscript?: boolean;  // Hide from manual (notes-only) sessions
  directive: string;       // The full system prompt directive for this action
}
```

Built-in actions:

| Action | Tool usage | Behavior |
|--------|-----------|----------|
| `summarize` | `update_title` + `save_to_notes` | Structured summary with bold section labels. Tries to set title if generic. `requiresTranscript: true`. |
| `key-points` | `save_to_notes` (append) | 5-15 bullet points, appended to existing notes |
| `action-items` | `save_to_notes` (append) | Numbered action items, appended |
| `meeting-minutes` | `update_title` + `save_to_notes` | Professional minutes format. `requiresTranscript: true`. |

When no action is selected (freeform chat), `GENERAL_DIRECTIVE` from `ai-prompts.ts` is used as the fallback directive.

`getActionsForSession(sessionType)` filters actions — manual sessions hide `requiresTranscript` actions (summarize, meeting-minutes) since there's no transcript to process.

`AIActionType` is `string` (not a closed union), so custom action IDs pass typecheck without modification.

**Adding a custom action** requires only creating an `ActionDefinition` object with a `directive` string and passing it via the `actions` prop on `AIContextProvider`. No core files need modification.

### Tool Registry (`ai-tools.ts`)

Tools are self-contained `ToolDefinition` objects registered at module load. Each has:
- **`schema`** — OpenAI `ChatCompletionTool` (JSON Schema for the function parameters)
- **`execute(args, ctx)`** — Performs the mutation, captures previous state for undo, returns `ExecutedTool`
- **`undo(undoData, ctx)`** — Reverts the mutation using captured state
- **`affects`** — Array of `ToolEffect` categories for declarative post-execution refresh

```typescript
type ToolEffect = "session-meta" | "notes";

interface ToolDefinition {
  schema: ChatCompletionTool;
  execute: (args, ctx: ToolContext) => Promise<ExecutedTool | null>;
  undo: (undoData, ctx: { sessionId: string }) => Promise<void>;
  affects?: ToolEffect[];
}
```

Built-in tools:

| Tool | Params | Effects | What it does |
|------|--------|---------|-------------|
| `update_title` | `{ title: string }` | `session-meta` | Sets session title (max 80 chars). Undo restores previous title. |
| `save_to_notes` | `{ content: string, mode: "replace" \| "append" }` | `notes` | Converts markdown → HTML via `marked`, saves/appends to notes, creates version snapshot. Undo restores previous note content. |
| `pin_session` | `{ pinned: boolean }` | `session-meta` | Pins/unpins session (only toggles if state differs). Undo restores previous pin state. |

**Post-tool refresh** uses `getToolEffects(toolNames)` to determine what to refresh based on `affects` metadata:
- `"session-meta"` → refresh sessions list + viewSession
- `"notes"` → increment note refresh counter

This replaces hardcoded tool name checks in `NoteDetailView.handleToolsExecuted`, so new tools with `affects` tags automatically trigger the right refreshes.

**Adding a new tool** requires only a `registerTool({...})` call in `ai-tools.ts` with an `affects` tag. No changes needed in `FloatingChatBar.tsx`, `NoteDetailView.tsx`, or `AIChatMessage.tsx`.

### Tool Context

Before executing tools, `FloatingChatBar` fetches a `ToolContext` via the provider's `getToolContext()`:
```typescript
{ sessionId, currentTitle, currentNote: DbNote | null, isPinned: boolean, segments?: DbSegment[] }
```
This allows tools to capture previous state for undo and make conditional decisions (e.g., `pin_session` only calls `togglePin` if the current state differs from the requested state). `segments` are used by `save_to_notes` to convert `[[seg:ID]]` citations in the saved content to interactive `<span>` elements.

### Prompt Assembly (`ai-prompts.ts`)

The prompt system is **directive-first** — the caller passes a `directive: string` (from an `ActionDefinition` or `GENERAL_DIRECTIVE`) rather than an action ID.

```typescript
getSystemPrompt(directive: string, transcriptText, noteText, attachments) → string
getSystemPromptWithToolContext(directive: string, ..., sessionMeta) → string
```

Assembly layers:
1. **Directive** — the action-specific system prompt (from `ActionDefinition.directive` or `GENERAL_DIRECTIVE`)
2. **Citation instruction** — appended only when transcript is present (instructs `[[seg:ID]]` format)
3. **Notes guidance** — always appended (instructs append vs. replace mode)
4. **Context sections** — `## Session Transcript`, `## Notes`, `## Attached Files` (only when non-empty)
5. **Session metadata** — current title, pin state, notes presence (via `getSystemPromptWithToolContext`)

For multi-session contexts, `getMultiSessionSystemPrompt(sessionsContext, attachments, folderContext?)` builds a read-only prompt with optional `FolderContextLayer[]` hierarchy (folder name + description pairs from root to leaf).

### Streaming with Tool Calls

`streamChatWithTools()` is an async generator that yields typed `StreamEvent`s:
- `{ type: "token", content }` — Text delta from the model (rendered incrementally)
- `{ type: "tool_calls", calls }` — Accumulated tool calls (emitted after stream ends, before `done`)
- `{ type: "done" }` — Stream complete

Tool call deltas arrive incrementally during streaming (partial JSON arguments across multiple chunks). The generator accumulates them by index in a Map, then parses the complete JSON arguments after the stream finishes. This is a **single-turn** tool flow — we execute tools locally and don't send results back to the model for a follow-up response.

### Undo System

Every tool execution captures `undoData` (the previous state). After tools execute:
1. A toast appears: "Session updated: Title, Notes [Undo]"
2. The undo window lasts 10 seconds (`UNDO_TIMEOUT_MS`)
3. Clicking Undo calls `undoToolCalls()` which reverses tools in LIFO order
4. The `[tool:...]` badge lines are stripped from the persisted message
5. Post-tool refresh fires via `onToolsExecuted` → `getToolEffects()`

### Transcript Citations

Segments are passed to the AI with IDs embedded in the timestamp: `[seg:abc123 0:30] Hello everyone...`. The system prompt instructs the AI to cite segments using `[[seg:ID]]` markers. In `AIChatMessage`, these are parsed and rendered as:
- Clickable pill buttons showing the timestamp (e.g., "1:15")
- Tooltip on hover showing the segment text preview (first 80 chars)
- Click seeks audio to that segment's `audio_offset_seconds` via `handleSeek`

In saved notes (via `save_to_notes` tool), citations are converted to `<span data-segment-ref>` elements that the Tiptap `SegmentReference` extension renders as interactive pills.

### Tool Badge Persistence

When tools execute, badge lines are prepended to the assistant message content:
```
[tool:update_title] Q1 Budget Planning Meeting
[tool:save_to_notes] Notes saved

Here's a summary of your session...
```
These persist in `chat_messages` DB, so they render as badges on reload. `AIChatMessage.parseToolBadges()` splits them from the text body. If the remaining text is empty (tool-only response), the message bubble is hidden — only badges show.

### Context Provider Pattern

`AIContextProvider` wraps `FloatingChatBar` and provides `AIContextValue` via React context. Factory functions in `ai-context.ts` create sources, tools, actions, and system prompt builders for different contexts:

- **Session context** (`createSessionSources`, `createSessionTools`, `createSessionSystemPromptBuilder`) — Single session with toggleable transcript and notes sources. All three tools available. Actions filtered by `getActionsForSession(sessionType)`.
- **Multi-session context** (`createMultiSessionSources`, `createMultiSessionTools`, `createMultiSessionSystemPromptBuilder`) — Folder-level AI chat aggregating sessions. No tools available (read-only analysis). Folder context hierarchy passed as `FolderContextLayer[]`.
- **Dictation context** (`createDictationSources`, `createDictationSystemPromptBuilder`) — Dictation history AI chat. Read-only analysis of dictation entries. No tools available.
- **List context** — `resolveListContext(filter, sessions, folders)` returns a `ListChatContext` (contextKey, sources, tools, systemPromptBuilder, placeholder) for any `ListFilter` type. `ListContextBar` renders the `AIContextProvider` + `FloatingChatBar` for non-session views.

Sources are toggleable via `toggleSource()` — users can enable/disable transcript and notes context independently. The `contextKey` (from `chatContextKey()`) determines chat message identity — switching sessions or folders resets the chat history. `AIContextValue` includes a `placeholder` string for context-specific input hints.

`SystemPromptBuilder` takes `(directive: string, contextParts, attachments)` — the directive is resolved by `FloatingChatBar` from the `ActionDefinition` before calling the builder.

### Markdown → HTML

Notes content in Tiptap is HTML. The AI generates markdown. Conversion uses `marked` (the `markdownToBasicHtml` function). Prompts explicitly instruct the AI to use `**bold text**` for section labels instead of `#` headings, because Tiptap renders `<h1>`/`<h2>` tags disproportionately large in the notes editor.

### Provider Compatibility

The system uses the OpenAI SDK with configurable `baseURL`, supporting:
- **OpenAI** (direct) — Full tool calling support
- **OpenRouter** — Routes to Claude, GPT, etc. with OpenAI-compatible tool calling
- **Custom** — Any OpenAI-compatible endpoint (e.g., local Ollama, LM Studio)

OpenRouter requests include `HTTP-Referer` and `X-Title` headers per their requirements.

### Extensibility

The system is designed so that user-created actions and tools require minimal file changes:

**Custom action** — just data:
```typescript
const customAction: ActionDefinition = {
  id: "my-analysis",
  label: "My Analysis",
  description: "Custom analysis prompt",
  icon: Sparkles,
  directive: "You are a specialized analyst that...",
};
// Pass via actions={[...builtIns, ...customActions]} to AIContextProvider
```

**Custom tool** — one `registerTool()` call in `ai-tools.ts`:
```typescript
registerTool({
  affects: ["session-meta"],
  schema: { type: "function", function: { name: "my_tool", ... } },
  execute: async (args, ctx) => { /* ... */ },
  undo: async (undoData, ctx) => { /* ... */ },
});
// Automatically picked up by getRegisteredTools() and getToolEffects()
```

### Key Design Decisions

1. **Self-contained action definitions** — Each `ActionDefinition` carries its own `directive` string. No separate lookup table or closed enum. Custom actions are just data objects.
2. **`AIActionType` is `string`** — Not a closed union. Any action ID passes typecheck. Built-in IDs documented in JSDoc.
3. **Directive-first prompt assembly** — `getSystemPrompt` takes a `directive: string`, not an action ID. The caller resolves the directive from the action definition. This decouples prompt construction from the action registry.
4. **Tool effect categories** — `affects: ToolEffect[]` on each `ToolDefinition` declares what side effects a tool has. `getToolEffects()` aggregates effects from executed tools. Post-tool refresh is driven by effects, not hardcoded tool names.
5. **Single-turn tool calling** — We don't send tool results back to the model. The AI calls tools, we execute them, and prepend badge metadata to the message. This avoids an extra API round-trip and works well because our tools are simple mutations with clear success/failure.
6. **Modular tool registry** — Tools are self-contained objects with schema + executor + undo + affects. Adding a tool touches one file. The registry pattern avoids a switch statement in the execution path.
7. **Undo via captured state** — Each tool captures the previous value (not a diff). This makes undo reliable even if other mutations happen between execute and undo.
8. **Badge lines in message content** — Tool metadata is stored as `[tool:name] detail` prefix lines in the DB message content. This is a simple, persistent format that survives reload without a separate DB column or join table.
9. **`marked` for markdown→HTML** — Replaced a hand-rolled line parser with `marked` for correct handling of all markdown edge cases (nested lists, inline formatting, etc.).
10. **Bold instead of headings for notes** — AI prompts explicitly forbid `#` headings because Tiptap's StarterKit renders them oversized in the notes editor. `**Bold**` labels provide visual structure without layout problems.
11. **Segment IDs in transcript context** — The `[seg:ID timestamp]` format lets the AI reference specific segments. Citations are rendered as interactive chips, not raw text. The AI is instructed to cite selectively.
12. **Modular AI context via factory functions** — Session and multi-session contexts use the same `FloatingChatBar` component but different sources/tools/prompts, assembled via factory functions in `ai-context.ts`. Adding a new context type (e.g., cross-folder analysis) only requires new factory functions.

## Keyboard Shortcuts

### Two-Tier Architecture

Shortcuts are split into two tiers with different mechanisms:

**In-app shortcuts** (`useKeyboardShortcuts`, mounted in `AppLayout`):
- Capture-phase `keydown` listener on `document`
- `mod+key` format (where `mod` = Cmd on macOS, Ctrl on Windows)
- Suppressed when input/textarea/contenteditable is focused
- 11 actions across Recording, Navigation, and Editor categories

**Global shortcuts** (`useGlobalShortcuts`, mounted in `App.tsx`):
- `@tauri-apps/plugin-global-shortcut` — work even when app is unfocused
- `CmdOrCtrl+Key` format (Tauri's platform-independent modifier)
- 4 static shortcuts (new session, new session with backfill, stop recording, new note) + dynamic dictation shortcuts from `dictation.slots`
- `focusWindow()` brings app to front before executing actions

### Registry (`lib/shortcuts.ts`)

Static `SHORTCUTS` array defines 17 shortcuts across 5 categories (`ShortcutCategory`): Recording, Navigation, Editor, General, Dictation. Each `ShortcutDefinition` has `id`, `label`, `description`, `category`, `defaultBinding`, optional `isGlobal` and `isDictation` flags.

`SHORTCUT_MAP` provides fast lookup by ID. `getBinding(id, overrides)` returns the user override or default binding.

### Override System

`shortcutBindings: Record<string, string>` in persisted settings maps shortcut IDs to custom bindings. `ShortcutsTab` provides a capture-mode UI for rebinding. A shared `shortcutCaptureActive` ref prevents shortcuts from firing while the user is rebinding.

### Binding Conversion

- `eventToBinding(e: KeyboardEvent)` → `"mod+shift+k"` format (in-app)
- `eventToGlobalBinding(e: KeyboardEvent)` → `"CommandOrControl+Shift+N"` format (Tauri global)
- `formatShortcutDisplay(binding)` → platform-specific symbols (⌘⇧N vs Ctrl+Shift+N)

### Custom Events

Decoupled side effects via custom DOM events:
- `yapstack:toggle-chat` — Toggles the floating chat bar
- `yapstack:toggle-search` — Toggles the command palette
- `yapstack:confirm-delete-session` — Triggers delete confirmation dialog
- `yapstack:dictation-start` — Starts dictation recording (with `{ slotId }` detail)
- `yapstack:dictation-stop` — Stops dictation recording
- `yapstack:dictation-idle` — Signals dictation has returned to idle (used for toggle mode cleanup)

## Dictation

### Overview

Voice dictation via global shortcuts. Supports two activation modes: hold-to-talk (press and hold to record, release to stop) and toggle (press once to start, press again to stop). Dictation operates independently of the main recording feature but shares the WhisperClient (V1 limitation: unavailable during active recording).

### Activation Modes

`DictationActivationMode: "hold" | "toggle"` — configurable in settings.

- **Hold mode** (default): Global shortcut Pressed → start recording, Released → stop and transcribe. Natural for short dictation.
- **Toggle mode**: First press → start recording, second press → stop and transcribe. Toggle state tracked via module-level `toggleActiveSlots: Set<string>` in `useGlobalShortcuts`. On stop, dispatches `yapstack:dictation-idle` for cleanup after processing completes.

### No-Input Detection

When recording exceeds 3 seconds with no speech detected (transcription returns empty), the dictation bubble shows a `"no-input"` state (yellow ring, pulsing animation) before returning to idle.

### Dictation History

Dictation entries are persisted to SQLite via the `dictation_history` table (DB migration v10). Each entry stores: slot metadata (name, AI config, output action), input/output text, optional WAV file path and duration, and optional session ID correlation.

- **DB CRUD**: `insertDictationHistory`, `listDictationHistory`, `getDictationHistory`, `deleteDictationHistory`, `clearDictationHistory`, `updateDictationHistorySessionId` in `lib/db.ts`
- **Store**: `dictationHistory` state with `loadDictationHistory`, `deleteDictationHistoryEntry`, `clearDictationHistoryEntries` actions
- **WAV correlation**: On `SESSION_WAV_READY` event, matches by UUID to update the history entry's `session_id`
- **UI**: `DictationHistoryList` (grouped by day, clear all button) + `DictationHistoryCard` (badges for slot/AI/action, WAV playback, context menu with delete/move-to-note)
- **Navigation**: "Dictation" button in `AppSidebar` sets `ListFilter { type: "dictation" }`. `NoteCardList` routes to `DictationHistoryList` when filter is dictation.

### Dynamic Slots

Dictation uses configurable slots (not a fixed count). Each `DictationSlot` has:
- `id` (UUID), `name`, `enabled` flag
- `aiEnabled` + `prompt` — optional AI post-processing via OpenAI-compatible API
- `outputAction: "paste" | "clipboard" | "new-note"` — determines where transcribed text goes

Slots are stored in `dictation.slots` in persisted settings. Keybinds are stored in the existing `shortcutBindings` map (keys: `global.dictation-{slotId}`). `useGlobalShortcuts` subscribes to both `shortcutBindings` and `dictation.slots` for re-registration.

### State Machine

`useDictation` hook (mounted in `App.tsx`, main window only) manages the lifecycle:

```
idle → recording → transcribing → processing → done → idle
         ↑                                        ↓
     (key press)                              (500ms delay)
```

1. **recording**: Key pressed → validate (dictation enabled, engine ready, not in active recording) → show bubble, start timing
2. **transcribing**: Key released → `triggerInstantCapture()` for elapsed duration → `transcribeAudio()` via WhisperClient
3. **processing**: If `slot.aiEnabled && slot.prompt` → call AI API with transcription + system prompt
4. **done**: Route output based on `slot.outputAction`:
   - `"paste"`: `clipboard_paste(text, true)` — copies to clipboard + auto-pastes via osascript
   - `"clipboard"`: `clipboard_paste(text, false)` — copies to clipboard only
   - `"new-note"`: Creates a manual session, saves transcription as notes, opens in main window

### Dictation Bubble Window

Separate Tauri window (`?window=dictation`) configured as:
- 220×64px, transparent, no decorations, no shadow, always-on-top, skip taskbar, no focus steal
- Positioned at bottom-center of screen during dictation
- `DictationBubble` component shows state-dependent visuals: red ring (recording), blue (transcribing), purple (processing), green (done), yellow pulsing (no-input)
- `showBubble()` uses `commands.showOverlayPanel("dictation")`, `hideBubble()` uses `commands.hideOverlayPanel("dictation")` — platform-agnostic (macOS: NSPanel, others: WebviewWindow)
- `App.tsx` routes to `DictationBubble` when `?window=dictation` is detected, avoiding hook violations

### Backend Command

`clipboard_paste(text, auto_paste)` in `commands/dictation.rs`:
- Writes text to clipboard via `pbcopy` (macOS) / `clip` (Windows)
- If `auto_paste`: waits 50ms, then simulates Cmd+V via `osascript` (macOS)

## Analytics

Privacy-first usage analytics via [Aptabase](https://aptabase.com) — no user IDs, no fingerprinting. The Aptabase Tauri plugin automatically attaches app version (from `tauri.conf.json`), OS, and locale to every event.

### Setup

- **Rust**: `tauri-plugin-aptabase` registered in `lib.rs` with `env!("APTABASE_KEY")` (compile-time). On `RunEvent::Exit`, flushes pending events via `flush_events_blocking()`.
- **Frontend**: `@aptabase/tauri` JS SDK. All calls go through `src/lib/analytics.ts` — typed wrappers around `trackEvent()`, fire-and-forget (no awaiting).
- **ACL**: `aptabase:allow-track-event` in `capabilities/default.json`.
- **Build**: `APTABASE_KEY` sourced from `.env` (local) or GitHub Secrets (CI). Optional for dev — analytics silently disabled when unset.

### Event Taxonomy (~34 events)

| Category | Events | Key Props |
|----------|--------|-----------|
| App lifecycle | `app_launched`, `app_exited` | capture_source, model_size, theme, ai_provider, dictation_slot_count |
| Sessions | `session_created`, `session_stopped`, `session_deleted`, `sessions_cleared`, `manual_note_created` | source, trigger (sidebar/tray/shortcut), duration_seconds, segment_count |
| Dictation | `dictation_started`, `dictation_completed`, `dictation_failed`, `dictation_slot_created/deleted/configured`, `dictation_history_cleared`, `dictation_history_entry_deleted`, `dictation_moved_to_note` | slot_id, duration_ms, ai_processed, output_action, error_reason |
| AI chat | `chat_message_sent`, `chat_tool_executed`, `chat_tool_undone`, `chat_cleared` | context, action_id, tool_name |
| Navigation | `search_used`, `folder_created`, `session_pinned/unpinned`, `session_moved_to_folder` | — |
| Shortcuts | `shortcut_used` | shortcut_id |
| Model/Engine | `model_downloaded/deleted/switched`, `engine_error` | model_size, from_size, error, phase |
| Settings | `setting_changed`, `ai_provider_changed`, `ai_connection_tested` | setting_name, new_value, provider, success |
| Content | `audio_playback_started`, `segment_edited`, `segment_hidden` | duration_seconds |
| Stream health | `stream_health_event` | source, status |

### Design Principles

- **No content tracking**: Never log transcript text, note content, or AI messages. Only structural metadata (counts, durations, IDs).
- **Booleans as 0/1**: Aptabase props only support strings and numbers — no booleans.
- **Truncation**: Error messages capped at 100 chars to avoid PII leakage.
- **Fire-and-forget**: Analytics never blocks UI or throws. All `trackEvent` calls are caught silently.
- **Single file**: Adding a new event = add a typed export to `analytics.ts` + one call at the integration point. No other files need to change.

## Ring Buffer Design

The `AudioRingBuffer` is the performance-critical component:
- **Single-producer, single-consumer** (SPSC) pattern
- **Producer** (audio callback): writes via `UnsafeCell`, stores `write_pos` with `Release`
- **Consumer** (app thread): loads `write_pos` with `Acquire`, reads from buffer
- **Monotonic counter**: `write_pos` never resets (wraps via modulo), enabling `snapshot_since()`
- **Per-buffer format**: Each ring buffer is created with its device's native sample rate and channel count. `start_mic()` does not mutate `AudioManager.config` — the buffer carries its own format. All extraction methods read each buffer's `sample_rate()` / `channels()` and deinterleave multi-channel data to mono before mixing or WAV export.
- **Default config**: `capture_history_seconds` (Rust default 180, frontend default 300 via `bufferMaxSeconds`). Devices typically operate at 48kHz (mono for mic, stereo for system audio on macOS). Ring buffer size is calculated from the actual device rate. `DEFAULT_SAMPLE_RATE` (16kHz) in `yapstack_common::config` is a fallback when no buffer is active.

## IPC Protocol (Sidecar)

JSON-line protocol over stdin/stdout. Each message is a single JSON object followed by `\n`.

**Requests** (tagged union via `#[serde(tag = "type")]`):
- `{"type":"transcribe", "id": 1, "audio_path": "/path/to/file.wav", "language": "en", "initial_prompt": "prior context..."}`
- `{"type":"load_model", "id": 2, "model_path": "/path/to/ggml-small.bin"}`
- `{"type":"shutdown"}`

**Responses**:
- `{"type":"transcription", "id": 1, "text": "...", "segments": [...], "duration_ms": 500}`
- `{"type":"model_loaded", "id": 2}`
- `{"type":"error", "id": 1, "message": "..."}`
- `{"type":"progress", "id": 1, "percent": 0.5}`

## Platform Support

| Feature | macOS | Windows (x64 MSVC) | Linux |
|---------|-------|---------------------|-------|
| Microphone capture | cpal (CoreAudio) | cpal (WASAPI) | cpal (ALSA) |
| System audio capture | cpal loopback (always available) | cpal loopback (WASAPI) | Unavailable |
| Whisper transcription | whisper.cpp (Metal) | whisper.cpp | whisper.cpp |
| Entitlements | audio-input, screen-capture | N/A | N/A |

### Windows WASAPI notes

- **System audio loopback** uses WASAPI event-driven loopback capture. When no application is producing audio, the loopback stream delivers zero packets — this is expected WASAPI behavior, not a stream error.
- The stream health watchdog skips write-position stall detection for system audio on Windows to avoid false restart storms during silence. System streams rely solely on the cpal error callback flag for failure detection.
- Microphone privacy settings on Windows 10/11 can block desktop app access. Capture-start failures surface as `CaptureState::Error` with an error message visible in the title bar and via toast notification.

## Audio Playback

WAV files are stored in `$APP_DATA_DIR/audio/{session_id}.wav` after recording. Playback uses a custom URI scheme protocol:

```
audio-stream://localhost/{filename}.wav
```

### Why a custom protocol?
The `convertFileSrc()` asset protocol URL (`asset://localhost/...`) works for `<audio>` elements in production builds but fails cross-origin in dev mode (`localhost:5173` vs `asset.localhost`). The custom `audio-stream` protocol is registered in `lib.rs` and serves WAV files directly from the audio directory.

### Range request support
The protocol supports HTTP `Range` headers for audio seeking. Returns `206 Partial Content` for range requests with `Content-Range` header, or `200` with full file for non-range requests. Security: only `.wav` files served, no path traversal allowed.

### Playback sync
`AudioPlayer` provides time updates via `requestAnimationFrame`. `ChatView` highlights the active segment by matching `currentPlaybackTime` against `audio_offset_seconds`. Clicking a segment timestamp seeks the audio via `handleSeek`.

## Key Design Decisions

1. **Sidecar process for Whisper** — Isolates heavy native dependency (whisper.cpp/cmake) from the main app binary. Crash isolation. Can be updated independently.
2. **Lock-free ring buffer** — Audio callbacks cannot block. SPSC with atomics avoids mutex contention.
3. **DTO layer in Tauri commands** — Domain types don't derive `specta::Type`. DTOs add the TypeScript generation derive and convert via `From` impls.
4. **Feature flags for optional native deps** — `whisper` and `metal` on yapstack-sidecar keep the default build lightweight. System audio is always compiled in via cpal loopback (no feature flag).
5. **16-bit PCM WAV export** — WAV files are written at the device's native sample rate (e.g. 48kHz). The sidecar resamples to 16kHz mono before Whisper inference. 16-bit quantization is sufficient (-96 dB noise floor).
6. **Session tracking via write_pos** — Sessions record the ring buffer's monotonic write position at start, then `snapshot_since()` at end. No separate buffer needed. For long sessions (> buffer capacity), `SessionWavWriter` streams audio incrementally to disk every 300ms during the live transcription loop.
7. **Custom URI scheme for audio** — `audio-stream://` protocol serves WAV files with range request support, avoiding cross-origin issues with the default asset protocol during development.
