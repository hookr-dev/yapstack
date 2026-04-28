# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Changelog discipline

`CHANGELOG.md` is a first-class artifact and must be updated **in the same PR/commit** that makes a user-visible change. "User-visible" means anything a downstream consumer would notice: new commands/features, removed or renamed APIs, schema migrations, behaviour changes, perf wins they can feel, bug fixes worth calling out, dependency upgrades that affect compatibility.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) under `## [Unreleased]`:

- **Added** — new features.
- **Changed** — behaviour changes to existing features.
- **Deprecated** — soon-to-be-removed features.
- **Removed** — deletions.
- **Fixed** — bug fixes worth surfacing.
- **Security** — vulnerability fixes.

When cutting a release, rename `[Unreleased]` to the new version with the date (`## [1.0.0-alpha.6] - YYYY-MM-DD`) and start a fresh empty `[Unreleased]` block above it.

**Skip the changelog** for: pure refactors, internal test changes, formatting, doc-only edits that don't affect the API, dependency bumps that don't change behaviour. When in doubt, add an entry — it's cheap.

Treat a missing changelog entry on a user-visible PR as a review-blocking gap, the same way a missing test would be.

## Build & Test Commands

```bash
# Full verification (run before committing)
pnpm check                                       # Rust build + test + fmt + clippy + TS typecheck + ESLint + vitest

# Test
pnpm test                                        # Rust + frontend tests
pnpm test:frontend                               # frontend only (vitest)
pnpm test:rust                                   # Rust only (cargo test --all)
pnpm test:watch                                  # frontend watch mode

# Targeted Rust tests
cargo test -p yapstack-audio                        # single crate
cargo test -p yapstack-audio -- ring_buffer         # specific module
cargo test -p yapstack-audio -- --ignored           # hardware-dependent tests

# Lint
pnpm lint                                        # Rust fmt + clippy + ESLint
pnpm typecheck                                   # TypeScript type checking

# Feature-flag builds
cargo build -p yapstack-sidecar --features whisper                       # whisper-rs (requires cmake)
cargo build -p yapstack-sidecar --features parakeet                      # parakeet-rs + ort + Sortformer
cargo build -p yapstack-sidecar --features parakeet,coreml               # + ORT-CoreML EP
cargo build -p yapstack-sidecar --features parakeet,webgpu               # + ORT-WebGPU EP
cargo build -p yapstack-sidecar --features whisper,parakeet,metal,coreml,webgpu  # apple full

# Sidecar release/dev build (copies to apps/desktop/src-tauri/binaries/
# and mirrors to target/debug/yapstack-sidecar so iterative dev rebuilds
# take effect on the next sidecar respawn — see scripts/build-sidecar.sh)
./scripts/build-sidecar.sh
./scripts/build-sidecar.sh --dev

# Force a Parakeet ORT EP at runtime (overrides the Auto policy):
YAPSTACK_PARAKEET_ACCEL=cpu|coreml|webgpu npm run dev

# Frontend only
pnpm --filter @yapstack/desktop dev                 # vite dev server

# Full app
pnpm tauri dev
pnpm tauri build

# DMG packaging
./scripts/build-dmg.sh                           # standard build
```

## Architecture

Tauri v2 desktop app for real-time audio capture and transcription. Rust backend, React 19 + TypeScript frontend. Sessions, segments, notes, and folders persisted to SQLite via `tauri-plugin-sql`.

### Crate structure

- **yapstack-common** — Shared types, config, `deinterleave_to_mono()`, sidecar IPC protocol, and the `engines` catalogue (engine kinds × supported languages × capability flags consumed by both backend validation and the frontend dropdowns). No business logic.
- **yapstack-audio** — Audio capture engine. Lock-free SPSC ring buffers (`UnsafeCell` + atomics), mic/system capture via `cpal`, audio mixing, WAV export (16-bit PCM via `hound`), `SessionWavWriter` for incremental streaming WAV. Position-based extraction API consumed by live transcription (`buffer_positions`, `extract_since`, `extract_sources_since`, `peek_energy_rms`) — there's no separate "instant capture" or `start_session`/`end_session` surface anymore. Stream error detection via `Arc<AtomicBool>` in cpal error callbacks, stream restart without buffer loss.
- **yapstack-transcription** — Model management for Whisper (ggml, single file), Parakeet TDT (multi-file ONNX bundle in a per-variant directory), and Sortformer (single ONNX file for diarization), all from HuggingFace with streaming SHA-256 verification. `TranscriptionClient` spawns the sidecar with `--engine` and communicates via JSON-line IPC. `transcribe_with(audio_path, language, initial_prompt, diarization)` exposes the per-call diarization flag.
- **yapstack-sidecar** — Standalone binary. Spawned with `--engine whisper|parakeet`, optionally `--vad-model PATH` (Whisper-only) and `--sortformer-model PATH` (Parakeet-only). Dispatches IPC requests through the `TranscriptionBackend` trait in `engines/mod.rs`; concrete impls in `engines/whisper.rs` (whisper-rs + Metal) and `engines/parakeet.rs` (parakeet-rs + ONNX Runtime, plus Sortformer post-pass for speaker IDs). Both backends behind feature flags; default build includes both. Logging goes to stderr.
- **yapstack-desktop** (`apps/desktop/src-tauri`) — Tauri command layer. Thin wrappers converting domain types to DTOs that derive `specta::Type` for TypeScript generation. Unified `CommandError` type (`commands/error.rs`) for all commands. Live transcription controller with per-source VAD, backfill, prompt context windowing, streaming session WAV, stream health monitoring with auto-restart, and per-segment `speaker_id` propagation when Parakeet+Sortformer is active.

### Key patterns

**DTO boundary**: Domain types in library crates use only `serde`. Tauri commands in `apps/desktop/src-tauri/src/commands/` define separate DTO structs deriving `specta::Type` with `From<DomainType>` impls. This keeps `specta`/`tauri` deps out of library crates.

**Ring buffer**: `AudioRingBuffer` is SPSC with a monotonic `write_pos` counter (never resets). Producer uses `Release` ordering, consumer uses `Acquire`. `snapshot_since(pos)` lets the live-transcription loop pull only the audio written since its last cursor; the loop's `BufferPositions` cursors replace the older `start_session`/`end_session` model.

**Sidecar IPC**: Tagged JSON unions via `#[serde(tag = "type")]`. Request types: `transcribe` (with optional `initial_prompt`, optional `diarization: bool` honored by Parakeet only), `load_model`, `shutdown`. Response types: `transcription` (segments include `speaker_id: Option<u8>`), `model_loaded`, `error`, `progress`. Each request has a `u64` ID for correlation. The protocol is engine-agnostic; the sidecar's `--engine` flag picks the backend.

**State management (backend)**: Four `Arc<Mutex<T>>` states in Tauri — `AudioManagerState`, `ModelManagerState`, `TranscriptionClientState`, `LiveTranscriptionState`. `TranscriptionClientState` wraps `Option<TranscriptionClient>` (None until `init_transcription_client` is called). `LiveTranscriptionState` wraps `Option<LiveTranscriptionRuntime>` (controller + dynamic vocabulary hints; None until live transcription starts).

**State management (frontend)**: Zustand store (`stores/appStore.ts`) with persisted settings (**version 23**, with migrations). SQLite via `tauri-plugin-sql` for session/segment/note/folder/tag/chat-message/dictation-history persistence (`lib/db.ts`). Segment writes serialized via a promise queue to prevent backfill/live race conditions. Navigation model: views `"note-list" | "note-detail" | "settings"` with `ListFilter { type: "all" | "pinned" | "folder" | "dictation", folderId? }`. **DB schema at migration v15**: sessions, segments, folders, notes, note_versions, shares (dormant), chat_messages, session_folders, dictation_history, tags, session_tags, FTS5 search tables, and `session_audio_parts` (durable source of truth for session audio files). Pre-migration runtime patches in `db::ensure_runtime_schema()` create the `audio_save_locations` table (used by reconciliation on next startup) and sweep stale `recording`-status sessions left by a prior crash; the Parakeet+Sortformer `segments.speaker_id INTEGER` column is added by the frontend's `getDb()` after migrations run. Settings carry `selectedEngine` (Whisper | Parakeet — Whisper is the upgrade-safe default), `selectedParakeetVariant`, `diarizationEnabled`, and a per-session `speakerNames` map for renaming `Speaker N` → custom labels. Store also tracks `tags: DbTag[]` and `sessionTagMap: Record<string, string[]>` alongside the folder equivalents.

**Audio playback**: Custom `audio-stream://` URI scheme protocol (registered in `lib.rs`) serves session audio files (WAV or MP3) from each session's tracked audio directory with HTTP range request support for seeking. The `TrustedAudioDirs` allow-list is seeded from `session_audio_parts` rows + `audio_save_locations` at startup so user-chosen export paths still play.

**Frontend UI**: Notes-first model with `AppSidebar` + main content area. Completed transcriptions show split pane (`react-resizable-panels`) with transcript left and Tiptap rich text editor right. Drag-and-drop via `@dnd-kit`, search via `cmdk` (Cmd+K).

**AI chat & tool calling**: `FloatingChatBar` provides per-session AI chat using OpenAI-compatible APIs with multi-turn tool calling. Ten tools registered in `ai-tools.ts`: `update_title`, `save_to_notes` (modes: `replace` / `append` / `prepend` / `find_replace`), `pin_session`, `tag_session`, `add_session_to_folder`, `search_folders`, `search_sessions`, `search_dictations`, `get_session_context`, `replace_in_transcript`. Multi-turn execution loop sends tool results back to the LLM for phased workflows (classify folder → write summary). Action directives use two-phase tool chaining. Tool execution state rendered by `ToolExecutionBlock` with per-tool status indicators; undone tool calls render as grayed receipts rather than disappearing. Adding a new tool only requires a `registerTool()` call. See `docs/ARCHITECTURE.md` § "AI Chat & Tool Calling" for full details.

**Tags vs folders**: Folders are the primary organizational primitive — hierarchical, with descriptions that flow into AI context. Tags are flat, lightweight metadata applied by the AI during summarization. Auto-suggestion chips during recording suggest **folders only**. Tags stored in `tags`/`session_tags` tables (migration v11). See `docs/ARCHITECTURE.md` for the full design rationale.

**Analytics**: Privacy-first usage analytics via Aptabase (`tauri-plugin-aptabase` + `@aptabase/tauri`). ~35 events covering app lifecycle, sessions, dictation, AI chat, navigation, shortcuts, model/engine, settings, and stream health. All calls go through `src/lib/analytics.ts` — typed fire-and-forget wrappers. No content is ever tracked (no transcript text, no notes, no AI messages). Booleans sent as 0/1 (Aptabase constraint). Error strings truncated to 100 chars. `APTABASE_KEY` env var required at compile time — sourced from `.env` (local) or GitHub Secrets (CI). Adding a new event: add a typed export to `analytics.ts` + one call at the integration point.

**Feature flags**: yapstack-sidecar exposes `whisper` (whisper-rs, requires cmake), `parakeet` (parakeet-rs + ONNX Runtime via `ort`, also enables Sortformer diarization), `metal` (Metal acceleration for whisper-rs), `coreml` (Parakeet ORT-CoreML EP), `webgpu` (Parakeet ORT-WebGPU EP, Metal under the hood on macOS). Default features = `["whisper", "parakeet"]`; the dev/release build script adds `metal,coreml,webgpu` on Apple targets. The sidecar compiles with any subset; when the engine the user picks isn't compiled in, the dispatcher returns a clear error per request. System audio capture on macOS is always available via cpal loopback (output device capture).

**Parakeet acceleration policy**: at sidecar startup, the env var `YAPSTACK_PARAKEET_ACCEL=auto|cpu|coreml|webgpu` selects the ORT execution provider for `ParakeetTDT::from_pretrained`. Default `auto` prefers CoreML when compiled in *and* the model dir contains no external `.onnx.data` initializer files; otherwise CPU. This avoids the deterministic `model_path must not be empty` failure ORT-CoreML hits on Parakeet TDT v3 (which ships a 2.3 GB external `.data` blob). Per-spawn behaviour: any chosen accelerator that fails at load time is caught in `engines/parakeet.rs::load_model` and falls back to CPU with a `WARN` line, so the sidecar never returns "no model loaded". Empirical RTFx on a single Apple Silicon machine, Parakeet TDT v3, 2-13 s chunks: CPU 4-8×, WebGPU 4-9× (similar mean, higher variance), CoreML unavailable for this model. The path to a real (10×+) speedup is data-inlining + CoreML or a Swift sidecar with FluidAudio's ANE pipeline — see `feat/parakeet-engine` discussion.

### Live transcription

The core real-time feature. `commands/live_transcription.rs` runs an async loop:
1. **Transcription client extraction** — On start, the `TranscriptionClient` is taken from shared `TranscriptionClientState` and held privately in `TranscriptionContext.transcription_client`. Zero contention with other commands. Returned to shared state on exit (even after panics via `AssertUnwindSafe`).
2. **Streaming session WAV → part on disk** — If `config.session_id` is set, creates `SessionWavWriter` at `$AUDIO_DIR/{session_id}.{part_index}.wav` (`audioSaveLocation` if set, else `$APP_DATA_DIR/audio/`; `part_index = 0` for a fresh session, `N` when resuming a session that already has parts). Every 300 ms the loop extracts new audio via `extract_since()` and appends. On stop, the part is finalized in the user's `audioExportFormat` (WAV kept as-is, MP3 re-encodes at `mp3Bitrate` and deletes the source WAV) and the parent dir is registered with `TrustedAudioDirs` for playback. When `config.persist_audio_part` is true (the default — actual sessions), a `session_audio_parts` row is inserted from Rust *before* emitting `"session-part-ready"` so the DB stays the durable source of truth even if the FE listener is gone. Dictation passes `persist_audio_part: false` and stores the path on `dictation_history` instead — the synthetic per-utterance id has no `sessions.id` to FK against, and inserting would either error or strand orphans for `clearAllSessions` to sweep. Empty recordings emit `"session-wav-error"` and the empty file is deleted.
3. **Backfill** — On start, rewinds cursors by `backfill_seconds`, extracts historical audio, transcribes concurrently with the live loop. Emits segments with `is_backfill: true`. Emits `backfill-complete` event when done.
4. **Per-source VAD** — `SourceVadState` tracks mic and system audio independently. Speech probability comes from a single shared **Silero V5 ONNX session** (`commands/silero_vad.rs`, bundled in-binary via the `silero` crate — no runtime download). Each source owns a `SileroSource` with its own LSTM stream state and VAD-only read cursor independent of the chunk-extraction cursors. Every poll, raw mono samples are extracted per source, resampled to 16 kHz via `yapstack_common::audio::resample` (rubato sinc), and fed through Silero to emit one probability per 32 ms frame; the state machine iterates every frame so intra-poll speech (utterance that starts and ends inside one batch) isn't missed. Thresholds are Silero probabilities (0.5 speech / 0.35 end, hysteresis) for both engines. Timing knobs in `VadTuning` stay per-engine: Whisper honors the user's `silence_duration_ms` at a 300 ms poll with no pre-roll; Parakeet uses a fixed 200 ms silence window, 10 s max chunk, 100 ms poll, 250 ms pre-roll.
5. **Prompt context** — Two-tier prompting (Whisper only): each source maintains `accumulated_text` (up to 1000 chars internally), truncated to `prompt_context_chars` (default 350). The `initial_prompt` actually sent to Whisper is `<vocab_hints>. <accumulated_text>` — vocabulary hints are folder/tag names (≥4 chars, comma-separated, ~80 char budget) built by `buildVocabularyHints()` in `lib/transcription.ts` from fresh DB queries and updatable mid-recording via the `update_vocabulary_hints` Tauri command (writes through an `Arc<Mutex<String>>` on `LiveTranscriptionRuntime`). The Parakeet TDT decoder has no text-prompt input, so the live controller silently drops `initial_prompt` for Parakeet sessions (gated on `client.engine() == EngineKind::Whisper`); `accumulated_text` machinery still tracks state for potential future use.
6. **Prompt decay** — After `prompt_decay_silence_seconds` (default 5.0, 0 to disable) of all-source silence, both `shared_prompt` and per-source `accumulated_text` are cleared via `check_prompt_decay()`. Prevents stale context from causing hallucination after long pauses. Backfill seeding is one-shot (`prompt_seeded_from_backfill` guard prevents re-seeding after decay). When speech resumes, the prompt rebuilds naturally from new transcription.
7. **Hallucination filtering** — Sidecar's `should_include_segment()` filters empty text, special tokens (`[BLANK_AUDIO]`, `[MUSIC]`), low-confidence segments (< 0.4), and known hallucination patterns ("thank you", "thanks for watching", etc.) at marginal confidence (< 0.6).
8. **Stream health monitoring** — Two-layer detection: cpal error callback flag (instant) + `write_pos` stall watchdog (2s threshold). Up to 3 restart attempts per source, reusing existing ring buffer. Emits `"stream-health"` events with status (`restarted`, `restart_failed`, `restart_abandoned`).
9. **Speaker diarization** (Parakeet only) — When `LiveTranscriptionConfig.diarization` is true and the active engine is Parakeet, every transcribe call sets `diarization: true` in the IPC payload. The sidecar runs Sortformer post-pass on the same audio, maps speaker ranges onto produced segments by maximum-overlap, and returns `speaker_id: Some(N)`. The frontend groups consecutive same-speaker segments under a `Speaker N` label (renamable, persisted per-session in the Zustand `speakerNames` map) and threads `(SpeakerName)` prefixes into the AI system prompt so chat answers attribute statements correctly.

Shared `TranscriptionContext` (immutable clone of transcription client, app handle, config, start time, dynamic vocabulary hints) is passed to both backfill and live processing.

### Auto-folder suggestions

During live recording, `FolderSuggestionTracker` (`lib/auto-tag.ts`) scans transcript segments for folder name keywords and shows inline suggestion chips. Folder-only — tags are applied by the AI during summarization. On acceptance, vocabulary hints are pushed to Whisper via `update_vocabulary_hints` (Whisper-only path; Parakeet sessions skip the push since the decoder ignores `initial_prompt`). See `docs/ARCHITECTURE.md` for matching rules and data flow.

### Audio defaults

`AudioConfig` has a single field: `capture_history_seconds` (Rust default 180.0, but the frontend passes `bufferMaxSeconds` which defaults to 300). Each buffer is created with its device's native sample rate and channel count (typically 48kHz mono for mic, 48kHz stereo for system audio on macOS). All extraction methods read each buffer's `sample_rate()` / `channels()` and deinterleave to mono via `yapstack_common::audio::deinterleave_to_mono()` before mixing or WAV export. WAV files are always single-channel. The sidecar resamples to 16kHz mono before Whisper inference. WAV export clamps f32 to [-1.0, 1.0] before i16 conversion. Temp WAV files use prefix `yapstack_capture_` and persist after creation (caller cleans up). Mixed-source capture fails with `SampleRateMismatch` if mic and system buffers have different sample rates.

### Capture source routing

`AudioManager::start_capture(source, mic_device_id)` dispatches based on `CaptureSource`: `MicOnly` → `start_mic()`, `SystemOnly` → `start_system_audio()` (hard error if unavailable), `Mixed` → `start_all()` (degrades to mic-only with error message if system audio fails). Mic device is identified by stable cpal device id (string), not display name; the Tauri `start_capture` command takes a `CaptureSourceDto` and `mic_device_id: Option<String>`.

### whisper-rs v0.15 API

The API differs from older examples: `full_n_segments()` returns `c_int` (not Result), segments accessed via `state.get_segment(i) -> Option<WhisperSegment>`, timestamps are in centiseconds (×10 for ms), confidence approximated via `1.0 - segment.no_speech_probability()`. Flash attention enabled via `ctx_params.flash_attn(true)`. Additional params: `suppress_blank`, `suppress_nst`, `no_context`, `temperature_inc(0.2)`.

## Detailed docs

- **`docs/ARCHITECTURE.md`** — Data flow between crates, ring buffer design, sidecar IPC protocol, live transcription pipeline, AI chat & multi-turn tool calling, folder-first summarization, tags vs folders, auto-folder suggestions, vocabulary hints, frontend component tree. Start here for cross-cutting concerns.
- **`docs/API_REFERENCE.md`** — Exact function signatures, struct fields, error variants, Tauri command args/returns, tool schemas. Read before adding or modifying any public API.
- **`docs/DEVELOPMENT.md`** — Build issues, feature flags, sidecar compilation, test infrastructure, model/temp file paths, frontend dependencies.
- **`docs/IMPLEMENTATION_LOG.md`** — Context on *why* something was built a certain way, what trade-offs were made, phase-by-phase build history. Includes Phase 18 (knowledge management: tags, multi-turn tools, vocab hints, auto-folder suggestions).
