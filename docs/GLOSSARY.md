# Glossary

Canonical names for things in YapStack. Use these in code, commits, and tickets so search and grep are reliable.

## Capture & audio

- **Mic** — microphone capture stream from a `cpal` input device.
- **System audio** — speaker/output capture stream (loopback on macOS via cpal output device).
- **Mixed** — combined mic + system audio capture.
- **Capture source** — `MicOnly | SystemOnly | Mixed` (`CaptureSource` enum, `CaptureSourceDto` for IPC).
- **Ring buffer** — lock-free SPSC `AudioRingBuffer` (UnsafeCell + atomics). Producer is the cpal callback; consumer is the live transcription loop.
- **`write_pos`** — monotonic byte cursor on the ring buffer, never resets. Cursors snapshot relative positions for `extract_since(pos)`.
- **`peek_energy_rms`** — non-consuming RMS over the last N samples. Used by the recording-indicator UI.

## Transcription

- **Engine** — `Whisper | Parakeet`. User-selectable in settings; backed by separate sidecar features.
- **Sidecar** — `yapstack-sidecar` binary. Spawned by the desktop app; communicates over JSON-line IPC on stdin/stdout. Logs to stderr.
- **IPC protocol** — tagged JSON unions (`SidecarRequest::Transcribe`, `SidecarResponse::Transcription`, etc.). One `id: u64` per request for correlation.
- **VAD (voice activity detection)** — Silero V5 ONNX model, shared singleton. Per-source state machines emit speech/silence transitions.
- **Backfill** — historical audio from before recording started, transcribed concurrently with the live stream. Segments carry `origin: "backfill"` (and `is_backfill: true` for backwards compat). The scheduler drains backfill at the lowest priority, behind FinalFlush and Live.
- **Scheduler** — `TranscriptionScheduler`. Single-worker priority queue in front of the sidecar lane (`commands/transcription_scheduler.rs`). Priorities: `FinalFlush > Live > Backfill`, with mic/system round-robin at the live tier. Sole caller of `transcribe_with` during a session.
- **Final flush** — closing-words chunks emitted at session stop. Submitted at `FinalFlush` priority so they outrank pending Live and Backfill work and survive the stop path even if backfill is still draining.
- **Job origin** — `JobOrigin::Live | Backfill | FinalFlush`. Mirrored on the wire as `SegmentOrigin` in `LiveSegmentEvent.origin`.
- **Prompt context** — Whisper-only feature: prior text fed into the next inference window for continuity. Decays after silence.
- **Diarization** — speaker labelling. Parakeet + Sortformer post-pass only. Produces `speaker_id: Some(u8)` on segments.
- **Hallucination filter** — drops empty / token-only / known-pattern segments (e.g., `"thank you"`) at low confidence.

## Storage

- **Session** — one recording session. Has a UUID, status, and zero or more parts.
- **Segment** — one transcribed snippet with timestamps. Persisted in the `segments` table.
- **Part** — one continuous audio file segment of a session. A session has N parts (resumed sessions add parts). Stored as WAV (then optionally re-encoded to MP3 on stop). Tracked in `session_audio_parts`.
- **Note** — Tiptap rich-text doc associated with a session. `notes` table; versions in `note_versions`.
- **Folder** — primary organizational primitive. Hierarchical, with icons and colors. Folder names flow into AI vocabulary hints.
- **Tag** — flat metadata applied by AI during summarization. Lighter than folders.
- **Dictation** — short voice-to-text utterance, processed via a per-slot system prompt and routed to paste/copy/note. Distinct from sessions; persisted in `dictation_history`.
- **Slot** — named dictation configuration with its own keybind, mode (hold/toggle), AI prompt, and output action.

## Auto-updater & release

- **Updater pubkey** — public Ed25519 minisign key in `tauri.conf.json`. Signature verified against the bundled `.app.tar.gz.sig`.
- **`latest.json`** — Tauri update manifest produced by `release.yml`. Lives at `releases/latest/download/latest.json`.
- **Draft release** — GH release in draft state, artifacts uploaded but not visible to the public; URLs resolve only after publish.

## Frontend

- **Live controller** — the long-running async loop in `commands/live_transcription.rs` that pulls audio, runs VAD, dispatches to the sidecar, and emits segment events.
- **Pressure event** — telemetry log + frontend event marking real-time-factor (RTFx) and stream lag. Used to detect "transcripts falling behind."
- **Vocabulary hints** — folder/tag-derived prompt prefix sent to Whisper; updated mid-recording via `update_vocabulary_hints`.

## CI & process

- **Verification** — `pnpm check`. The single command that gates merge.
- **CHANGELOG entry** — required for any user-visible change. Added under `## [Unreleased]`.
- **ADR (architecture decision record)** — short doc in `docs/adr/` capturing a structural decision and its rationale. Append-only; supersede rather than edit.
