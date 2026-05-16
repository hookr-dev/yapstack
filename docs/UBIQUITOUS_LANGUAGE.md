# Ubiquitous Language

The shared vocabulary for YapStack. Use these terms verbatim in code, docs, PRDs, issues, and UI copy. When a synonym appears in the "Aliases to avoid" column, prefer the canonical term.

## Capture

| Term                | Definition                                                                                                                | Aliases to avoid                  |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------- | --------------------------------- |
| **Capture**         | The act of acquiring raw audio frames from one or more devices into a ring buffer.                                        | Recording, listening              |
| **Capture source**  | Which audio inputs are being captured: `MicOnly`, `SystemOnly`, or `Mixed`.                                               | Input mode, audio source          |
| **Mic**             | The user's selected microphone input device.                                                                              | Microphone, input device          |
| **System audio**    | macOS loopback capture of the system output device.                                                                       | Loopback, desktop audio, speakers |
| **Ring buffer**     | The lock-free SPSC in-memory rolling audio store, sized by `capture_history_seconds`.                                     | Audio buffer, queue               |
| **Capture history** | The rolling window of audio retained in the ring buffer (default 300 s frontend / 180 s Rust).                            | Buffer window                     |
| **Stream**          | A single cpal input stream bound to one device. A capture may own a mic stream and/or a system stream.                    | Audio stream                      |
| **Stream restart**  | Recovering a cpal stream in place, reusing the existing ring buffer so no audio is lost. May rebind to the previously bound device (e.g. on a same-device retry) or to a different device (auto-failover); the resulting `bound_device_name` is surfaced on `stream-health`. | Reconnect, reset                  |
| **Stream health**   | The frontend-visible status of stream supervision: `restarted`, `restart_failed`, `restart_abandoned`.                    | Stream state                      |
| **Auto-failover**   | A **Stream restart** triggered by an OS device-change event (default-device change or a previously bound device leaving the device list), which re-binds the affected **Source** to the new system default. Drives the "Switched to {name}" toast on the FE. | Failover, hot-swap                |
| **Device broker**   | The always-on Tauri-side task that owns the receiving end of the audio crate's runtime-agnostic device-event sink. Debounces bursty Core Audio listener events in a 250 ms window, emits `devices-changed` to the FE, and dispatches **Auto-failover** restart intents through the live-transcription loop (or directly to `AudioManager` when no live session is active). | Device manager                    |

## Sessions and segments

| Term                  | Definition                                                                                                            | Aliases to avoid          |
| --------------------- | --------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| **Session**           | A bounded recording with its captured audio and ordered list of segments, persisted in SQLite.                        | Recording, take           |
| **Segment**           | A single transcribed utterance with start/end timestamps, text, confidence, and optional speaker ID.                  | Chunk, transcript line    |
| **Backfill**          | Re-transcription of audio captured *before* live transcription started, emitted as `origin: "backfill"` segments.     | History, replay, prefill  |
| **Session audio**       | The umbrella term for a session's persisted audio. A session's audio is composed of one or more **Session audio parts**; each part is one file in the user's chosen format. | Audio file, recording file |
| **Session audio part**  | One ordered slice of a session's audio (`part_index = 0, 1, 2…`), persisted as one file at `{audio_dir}/{session_id}.{part_index}.{wav\|mp3}` and recorded as one row in `session_audio_parts`. A fresh session has `part_index = 0`; resuming appends `part_index = N`. The DB row is the durable source of truth — written from Rust at finalize time before any FE event. | Audio chunk, segment audio |
| **Resume**              | Continuing a paused/stopped session by appending a new **Session audio part** rather than overwriting. Segments and parts both continue numbering from where the prior run left off. | Re-open, re-record         |
| **Session WAV**         | The WAV form of a single **Session audio part** (16-bit PCM mono). Always streamed to disk during recording; survives finalization only when **Audio export format** is `wav`. | Wav file                   |
| **Session MP3**         | The MP3 form of a single **Session audio part**, produced at finalization by encoding the streamed WAV and **deleting** the WAV. The part row keeps only the `.mp3`. | Compressed audio           |
| **Audio export format** | The user's choice of persisted session-audio encoding: `wav` or `mp3`. Applied at part finalization (and to user-triggered re-saves).             | Output format, save format |
| **MP3 bitrate**         | The user-configurable encode quality for the MP3 form (8–320 kbps).                                                                                  | Quality, kbps              |
| **Audio save location** | The user-overridden filesystem path where session audio parts are written (default `$APP_DATA_DIR/audio/`). Tracked in `audio_save_locations` so reconciliation can recover orphans on next startup. | Export path                |
| **Trusted audio dir**   | A directory in the runtime allow-list that the `audio-stream://` handler is willing to serve files from. Seeded at startup from `session_audio_parts.file_path` parents and `audio_save_locations`, then extended at finalize time. | Audio root                 |

## Transcription engines

| Term                | Definition                                                                                                          | Aliases to avoid                |
| ------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| **Engine**          | A transcription backend the sidecar can run: `Whisper` or `Parakeet`. Selected per session.                         | Backend, model type, provider   |
| **Variant**         | A specific weights bundle for an engine, e.g. a Parakeet TDT v3 directory.                                          | Model size, flavor              |
| **Model**           | The downloaded weights on disk: a single ggml file (Whisper) or a multi-file ONNX directory (Parakeet, Sortformer). | Weights, checkpoint             |
| **Sidecar**         | A standalone worker process spawned by the desktop app that speaks JSON-line IPC. Generic pattern; today the only sidecar is the transcription sidecar. | Worker, helper process          |
| **Transcription sidecar** | The `yapstack-transcription-sidecar` binary that hosts Whisper or Parakeet for one session.                   | Whisper sidecar, ASR worker     |
| **Embedding sidecar** | Reserved name (`yapstack-embedding-sidecar`) for the planned embedding worker. Not yet implemented.              | Vectorizer, embedder            |
| **Execution provider** | The ORT runtime backend for Parakeet: `cpu`, `coreml`, or `webgpu`. Chosen per spawn via `YAPSTACK_PARAKEET_ACCEL`. | Accelerator, EP only            |
| **Transcription client** | The Rust-side handle that owns the sidecar process and sends transcribe/load/shutdown requests.                | Whisper client (legacy alias)   |

## Live transcription

| Term                  | Definition                                                                                                                  | Aliases to avoid          |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| **Live transcription** | The async loop that polls the ring buffer, runs VAD, dispatches chunks to the sidecar, and emits segments in real time.    | Streaming transcription   |
| **VAD**               | Voice Activity Detection. YapStack uses Silero V5 ONNX, one shared session, with per-source LSTM state.                     | Speech detection          |
| **Source**            | One audio origin in the live loop, either `mic` or `system`. Each source has independent VAD state and cursors.             | Channel, track            |
| **Chunk**             | A contiguous span of speech extracted from one source between VAD start and end, sent as one transcribe request.            | Segment (in raw audio)    |
| **Pre-roll**          | Audio retained before VAD start to avoid clipping the first phoneme (Parakeet: 250 ms; Whisper: 0).                         | Lookback, lead-in         |
| **Initial prompt**    | The text fed to Whisper as decoding context. Honored only by Whisper; Parakeet has no text prompt input.                    | Prompt, context prompt    |
| **Prompt context**    | The most recent transcribed text retained per source as the next initial prompt (default 350 chars).                        | Context window            |
| **Prompt decay**      | Clearing prompt context after a configurable all-source silence window so stale context can't seed hallucinations.          | Prompt reset, context expiry |
| **Hallucination filter** | The sidecar's engine-aware reject pass: drops `[BLANK_AUDIO]`, low-confidence noise (< 0.4 always; 0.4–0.6 when marginal), and known phantom phrases. Whisper uses an aggressive always-reject list ("thank you", "thanks for watching"); Parakeet demotes the same phrases to marginal-only. | Output filter, blacklist  |

## Diarization

| Term             | Definition                                                                                                  | Aliases to avoid       |
| ---------------- | ----------------------------------------------------------------------------------------------------------- | ---------------------- |
| **Diarization**  | Assigning each segment to a speaker. Available only when engine is Parakeet and `diarizationEnabled` is on. | Speaker separation     |
| **Sortformer**   | The ONNX speaker-diarization model that runs as a post-pass after Parakeet transcription.                   | Diarization model      |
| **Speaker ID**   | A small integer (0, 1, 2…) assigned by Sortformer to a contiguous audio range.                              | Speaker index, voice ID |
| **Speaker label** | The display name for a speaker — defaults to `Speaker N`, user-renamable per-session.                      | Speaker name (ok)      |

## Dictation

| Term                       | Definition                                                                                                              | Aliases to avoid          |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| **Dictation**              | A short voice-to-text capture optimized for dropping text into the active app or clipboard. Runs through the same live-transcription pipeline as a session, under a synthetic dictation id (not a real `sessions.id`), and finalizes one **Session audio part** at `{dictation_id}.0.{wav\|mp3}`. The audio is kept on the `dictation_history` row regardless of **Output action**; only cancel deletes it.                | Quick note, voice command |
| **Dictation slot**         | A named, reusable dictation preset. Carries id, name, enabled flag, AI-enabled flag, prompt, output action, and default binding. The slots array is unlimited; one slot ("Raw Dictation") ships by default. | Preset, profile           |
| **Dictation activation mode** | How a slot's hotkey behaves: `hold` (push-to-talk, recording while held) or `toggle` (press to start/stop).          | Trigger mode              |
| **Output action**          | What to do with a finished dictation: `paste` into the focused field, `clipboard`, or `new-note` (create a session).    | Insertion mode, sink      |
| **Dictation history**      | Persisted log of past dictation outputs, distinct from sessions and exposed in the sidebar's `dictation` list filter.   | Recents                   |
| **Volume duck**            | Temporarily lowering the system output volume during a dictation so the user can hear themselves over earphone playback. Snapshots `(device_id, prior_level)` at apply and restores that *original* device on release, so a default-output change mid-duck (AirPods connect, USB DAC unplug) doesn't strand the original device at the ducked level. Only ever lowers — never raises. macOS only; no-op elsewhere. | Volume lowering, attenuation |
| **Duck target**            | The volume level (0.0–1.0) the system output is reduced to during a **Volume duck**. Persisted as `dictationDuckTarget`. | Duck level, ducked volume |

## Shortcuts and permissions

| Term                         | Definition                                                                                                  | Aliases to avoid           |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------- | -------------------------- |
| **Shortcut**                 | A named, user-rebindable in-app action (e.g. `command-palette`, `new-note`, `filter-all`).                  | Hotkey (in app), command   |
| **Binding**                  | The actual key combination assigned to a shortcut or dictation slot.                                        | Keybind, accelerator       |
| **Default binding**          | The factory-assigned binding for a shortcut or dictation slot before user customization.                    | Default keybind            |
| **Global hotkey**            | A system-wide keybinding registered with the OS (used by dictation slots even when YapStack is unfocused).  | System shortcut            |
| **Command palette**          | The Cmd+K overlay that searches sessions, notes, segments, folders, and runs shortcut actions.              | Quick switcher, Cmd+K menu |
| **Screen capture permission** | The macOS TCC grant required to capture system audio loopback on recent macOS versions.                    | Screen recording perm      |

## Notes and organization

| Term             | Definition                                                                                                                    | Aliases to avoid               |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------ |
| **Note**         | A rich-text document attached to exactly one session (UNIQUE `notes.session_id` FK), stored as HTML and edited via Tiptap.    | Document, markdown, write-up   |
| **Note version** | A historical snapshot of a note's content.                                                                                    | Revision, history entry        |
| **Folder**       | A user-created container for grouping sessions. Supports nesting. Folders contain sessions, not notes directly.              | Group, tag, collection         |
| **Pin**          | A boolean flag promoting a session into the "Pinned" list filter.                                                            | Star, favorite                 |
| **Share**        | A folder-scoped sharing record (table exists from migration v6, currently unused by the app).                                 | Export, public link            |
| **List filter**  | The sidebar's view scope: `all`, `pinned`, `folder`, or `dictation`.                                                          | View, tab                      |

## AI providers, connections, and profiles

| Term                  | Definition                                                                                                                                                                                                                                                  | Aliases to avoid                                                            |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| **AI provider**       | The *kind* of external LLM backend a **Connection** talks to: `openai`, `openrouter`, or `custom` (any OpenAI-compatible endpoint). A category, not an instance.                                                                                            | LLM provider, vendor, backend type                                          |
| **Connection**        | A configured, named instance of an **AI provider**: id, display name, kind, baseUrl, **Available models**, and an **API key** stored in app settings (OS keychain migration tracked as a follow-on PR). Users may have multiple connections of the same kind (e.g. "Work OpenAI" + "Personal OpenAI"). | Provider config, provider configuration, endpoint, account, provider slot   |
| **Profile**           | A user-named, thin selection of `(Connection, Model)` that features bind to via an **Assignment**. Carries no system prompt or sampling params — those stay on the consuming feature (e.g. `DictationSlot.prompt`).                                          | Preset, model profile, connection profile                                   |
| **Assignment**        | The binding between a **Feature consumer** and a **Profile** (or `null`, meaning "no AI for this feature"). Editable inline at each feature and summarised in the Profiles tab.                                                                              | Feature binding, target, route                                              |
| **Feature consumer**  | An in-app feature that needs a **Profile** to invoke an LLM: **Chat**, **Dictation slot** cleanup, notes generation, session summarization. Each feature consumer has one **Assignment**.                                                                    | Consumer, AI consumer                                                       |
| **Available models**  | The cached, filtered list of chat-capable models a **Connection** exposes via `/v1/models`. Populated at Connection save, refreshable manually, free-text override allowed in the **Profile** model picker.                                                  | Model catalog (legacy), model list, fetched models                          |
| **Model filter**      | The kind-aware predicate that drops non-chat models (embeddings, TTS, audio, moderation) from a Connection's raw `/v1/models` response. Applied for known kinds (`openai`, `openrouter`); `custom` Connections are unfiltered.                                | —                                                                           |
| **Quick start preset** | A pre-filled baseUrl template offered when creating a `custom` Connection (Ollama, LM Studio, llama.cpp, vLLM). Convenience only — no dedicated SDK or **AI provider** kind.                                                                              | Quickstart, URL template                                                    |
| **Slow hint**         | A cosmetic tag shown next to known reasoning-family models (`o1-*`, `o3-*`, `chatgpt-*`) in the **Profile** model picker. Purely informational — does not filter, does not affect routing.                                                                   | Slow tag, perf badge                                                        |
| **API key**           | A user-supplied secret authenticating one **Connection**. Stored on the Connection in app settings. Migrating to the OS keychain is a tracked follow-on PR; until then keys persist in plaintext alongside the rest of `AIConfig`. BYO-key per Connection. | Token, credential                                                           |
| **Chat profile picker** | The composer-header dropdown in **Chat** that overrides the assigned **Profile** for the current chat session only. The override is persisted on the chat session, so reopening that conversation resumes with its last-chosen Profile.                  | Model switcher, profile selector                                            |

## AI chat

| Term              | Definition                                                                                                                          | Aliases to avoid       |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| **Chat**          | A conversation with an LLM scoped to a context (a session, a folder, the pinned set, dictation history, or global), exposed via the floating chat bar. Uses the Chat **Assignment** as the default **Profile** for new chats; per-chat overrides are set via the **Chat profile picker** and persisted on the chat session. | Assistant, copilot     |
| **Context key**   | The string that scopes a chat: `global`, `pinned`, `dictation`, `folder:{id}`, or a session id. Determines what content the LLM sees and where messages are filed. | Scope, channel         |
| **Chat message**  | One turn (user or assistant) in a chat, persisted in SQLite with its context key.                                                   | Reply, exchange        |
| **Tool**          | A typed function the LLM can invoke via OpenAI tool calling. Each tool declares a `kind` (`"read"` or `"mutate"`) and an optional `affects` effect set; only `mutate` tools enter the Undo window. The current registry has ten: `update_title`, `save_to_notes`, `pin_session`, `tag_session`, `add_session_to_folder`, `replace_in_transcript` (mutating); `search_folders`, `search_sessions`, `search_dictations`, `get_session_context` (retrieval). | Action, function call  |
| **Citation**      | An inline `[[seg:ID]]` reference in chat output that resolves to a clickable timestamp chip on a segment.                           | Reference, link        |
| **Undo window**   | The 10-second period after a tool mutation during which the user can revert it.                                                     | Grace period, rollback |

## Lifecycle and states

| Term                | Definition                                                                                                   | Aliases to avoid             |
| ------------------- | ------------------------------------------------------------------------------------------------------------ | ---------------------------- |
| **Engine phase**    | The transcription engine's current readiness: `idle` → `downloading` → `initializing` → `ready` or `error`.  | Engine status, model status  |
| **Live phase**      | The live-transcription stream's UI state: streaming, stopped, or error.                                      | Streaming state              |
| **Session stopping** | The grace period after the user ends a session during which the WAV is finalized and the engine is released. | Finalizing                   |
| **Onboarding flow** | A named first-run setup sequence (e.g. picking an engine, granting permissions). Completion is timestamped.  | Setup wizard, first-run flow |
| **Update available** | The notification payload signaling a new app version is ready, with version and release notes.              | New version banner           |

## Audio mixing

| Term            | Definition                                                                                  | Aliases to avoid    |
| --------------- | ------------------------------------------------------------------------------------------- | ------------------- |
| **Mix config**  | The mic/system blend parameters applied when capture source is `Mixed`.                     | Mixer settings      |
| **Mic gain**    | The mic-channel multiplier in the mix config (1.0 = unity).                                 | Mic level, mic vol  |
| **System gain** | The system-audio-channel multiplier in the mix config.                                      | System level        |
| **Normalize**   | A flag that rescales the mixed output to avoid clipping after gains are applied.            | Auto-level          |

## Relationships

- A **Session** owns zero or more **Session audio parts** (one row per resume, ordered by `part_index` in `session_audio_parts`) and many **Segments**. Each part is exactly one file — a **Session WAV** *or* a **Session MP3**, never both — at the user's chosen **Audio export format** at finalize time. Choosing `mp3` re-encodes the streamed WAV and deletes it. Parts are concatenated at playback time, never on disk.
- A **Session** has zero or one **Note** (enforced by `UNIQUE` on `notes.session_id`); a **Note** has zero or many **Note versions**.
- A **Session** can belong to zero or many **Folders**, and a **Folder** can hold zero or many **Sessions** — many-to-many via the `session_folders` join table. **Folders do not contain Notes directly**; a note is reachable through its session.
- A **Capture** writes into one **Ring buffer** per **Source**; **Live transcription** reads from those buffers.
- An **Engine** runs inside the **Sidecar**; the **Transcription client** is its in-process handle. The engine's readiness is tracked as the **Engine phase**.
- **Diarization** assigns a **Speaker ID** to a **Segment**; the user maps it to a **Speaker label** per-session (Zustand-persisted, not in SQL).
- A **Chat** is scoped by a **Context key** (a session id, a folder id, `pinned`, `dictation`, or `global`) and resolves a **Profile** via the Chat **Assignment** by default; a per-chat-session override set through the **Chat profile picker** is persisted on the chat session and wins for that conversation. Session-scoped chats may emit **Citations** that resolve to that session's **Segments**.
- A **Profile** points to exactly one **Connection** and one **Model**. A **Connection** can be referenced by zero or many Profiles. A **Connection** belongs to exactly one **AI provider** kind, but a single AI provider can back many Connections.
- Deleting a **Connection** cascades to its **Profiles**, and onward to any **Assignments** pointing at those Profiles (which become `null`). Deleting a **Profile** cascades to its Assignments the same way. Both deletions are confirmation-gated and enumerate dependents; no orphan references are left behind.
- A **Connection** holds the **API key** (in app settings; OS keychain migration is a tracked follow-on) and **Available models**; the **Profile** picks one model from that list. Failures (auth, network, server down) surface as feature-level errors — no fallback chain is attempted.
- **Dictation** produces transcribed text without creating a **Session** unless the active **Dictation slot**'s **Output action** is `new-note`. Either way it is logged in **Dictation history** (with the finalized **Session audio part**'s path on `wav_file_path`; the `session_id` FK is set only on `new-note`).
- A **Dictation slot** owns one **Binding** (a **Global hotkey**), one **Dictation activation mode**, and a nullable **Profile** **Assignment** that controls AI cleanup (`null` = raw transcription with no AI pass; non-null = run cleanup through that Profile using the slot's **prompt**).
- **Capture source** = `Mixed` is the only mode that consults **Mix config** (mic gain, system gain, normalize).

## Example dialogue

> **Dev:** "When **live transcription** runs with **Mixed** capture, do mic and system share **VAD** state?"

> **Domain expert:** "No. Each **source** has its own Silero stream state and its own **chunk** cursor. They share the Silero ONNX session, but a mic utterance ending doesn't close a system utterance."

> **Dev:** "And the **initial prompt** — is that per-source too?"

> **Domain expert:** "Per-source on Whisper, yes — each source carries its own **prompt context**. On Parakeet the engine has no text prompt at all, so we drop it. **Prompt decay** still runs on both engines so we don't preserve stale state across long silences."

> **Dev:** "If **diarization** is on, when does the **speaker ID** show up on a **segment**?"

> **Domain expert:** "Only with the **Parakeet** engine. The **sidecar** runs **Sortformer** on the same chunk audio, picks the max-overlap speaker per segment, and returns it inline. The frontend then groups consecutive same-ID segments under a **speaker label** the user can rename."

> **Dev:** "And **backfill** segments — do they go through the same path?"

> **Domain expert:** "Same transcribe call, just submitted to the **scheduler** at `Backfill` priority and emitted with `origin: \"backfill\"`. Backfill seeds the prompt context once at start; after that, **prompt decay** can clear it like any other run."

> **Dev:** "When a **Session** finalizes, what determines whether we end up with a WAV or an MP3?"

> **Domain expert:** "The **Audio export format** setting. We always stream a **Session WAV** to disk during recording — that's the working buffer. At finalization, if the format is `wav`, we keep it. If it's `mp3`, we encode at the configured **MP3 bitrate** to produce a **Session MP3** and delete the WAV. Each recording run produces one such audio artifact — the **Session audio part** for that run. A session that's resumed N times ends up with N+1 parts; the **Audio player** stitches them in `part_index` order at playback time."

> **Dev:** "And **Dictation** — does it write to `session_audio_parts`?"

> **Domain expert:** "It uses the same live-transcription pipeline so the chunking, VAD, and streaming-WAV machinery are reused, but it sets `persist_audio_part: false` so finalize *skips* the `session_audio_parts` insert — that table is keyed on `sessions.id` and the dictation id is synthetic. Capture finalizes the file at `{dictation_id}.0.{wav|mp3}` regardless of **Output action**, and the path lands on the `dictation_history` row (`wav_file_path`, `wav_duration_seconds`) so the user can replay it. The difference between actions is what happens to the *text* — `paste` and `clipboard` route it out of the app, `new-note` also creates a real **Session** and links it via `dictation_history.session_id`. The audio is only deleted on cancel."

> **Dev:** "If I drag a **Session** into a **Folder**, does it leave its current folder?"

> **Domain expert:** "No — a session can sit in many folders at once. The relationship is many-to-many through the `session_folders` join table. Adding to a new folder doesn't remove it from any existing one; you have to remove it explicitly."

> **Dev:** "And the **Note** for that session — does it move with the folder, or have its own folder?"

> **Domain expert:** "A **Note** has no folder of its own. It's pinned 1:1 to a single **Session** by `notes.session_id`, and you reach it through the session. Folders contain sessions; notes ride along."

> **Dev:** "If a user wants their local llama.cpp for dictation cleanup but OpenAI for chat, do they create two **AI providers**?"

> **Domain expert:** "Two **Connections**, not two providers. The **AI provider** is just the kind — `custom` for the llama.cpp box, `openai` for the cloud one. They'd create one Connection of kind `custom` (Quick start preset pre-fills the llama.cpp URL) and one Connection of kind `openai`. Then they create two **Profiles** — one per Connection + chosen Model — and set the Dictation slot's **Assignment** to the local Profile and the Chat **Assignment** to the OpenAI Profile."

> **Dev:** "What if mid-chat they want to try the local Profile instead?"

> **Domain expert:** "Use the **Chat profile picker** in the composer. That overrides the Chat **Assignment** for that chat session only — it's persisted on the chat record, so reopening the conversation later resumes on the local Profile. New chats still start on whatever the Chat Assignment points to."

> **Dev:** "And if they delete the local Connection?"

> **Domain expert:** "Confirmation dialog enumerates the cascade: the Connection, every Profile that referenced it, and every Assignment that pointed at those Profiles. On confirm, all three layers are removed atomically. Any feature whose Assignment became `null` will show the inline 'no profile' empty state next time it's invoked. No silent breakage, no orphans."

## Flagged ambiguities

- **"Whisper client"** (legacy, no longer in code) vs **Transcription client** (current). The engine-agnostic Rust type is `TranscriptionClient` and the Tauri managed-state alias is `TranscriptionClientState`. The legacy `WhisperClient` / `WhisperClientState` aliases have been removed; "Whisper client" is dead terminology — always say **Transcription client**.
- **"Segment"** is overloaded: it means both a transcribed unit (the domain segment, persisted in SQLite) and, internally, a `WhisperSegment` from the whisper-rs API. In domain language, **Segment** always means the persisted transcribed unit. Use "whisper-rs segment" or "raw segment" when referring to the library type.
- **"Chunk"** vs **Segment**: a **chunk** is the *audio slice* sent to the sidecar between VAD start/end; a **segment** is what comes back transcribed. Don't use "chunk" to mean transcribed text.
- **"Source"** has two distinct meanings: (1) a **Capture source** enum (`MicOnly`/`SystemOnly`/`Mixed`) chosen at session start, and (2) a single audio origin (`mic` or `system`) within the live loop. The first is a *configuration*, the second is a *runtime entity* with its own VAD and cursor. Prefer **Capture source** for the enum and **Source** alone only inside the live-transcription context.
- **"Model"** vs **Variant** vs **Engine**: an **Engine** is the algorithm family (Whisper/Parakeet); a **Variant** is a specific weights bundle for that engine; a **Model** is the on-disk artifact. "Switching models" is ambiguous — say "switching engine" or "switching variant" depending on intent.
- **"Speaker name"** vs **Speaker label**: code uses `speakerNames` (the persisted Zustand map). In domain prose prefer **Speaker label** to make clear it's a user-facing display string mapped from a numeric **Speaker ID**.
- **"Recording"** is used loosely in UI copy. In domain language a **Session** is the persisted record and a **Capture** is the live act. Avoid "recording" as a noun unless quoting UI text.
- **"Session WAV"** vs **"Session audio"** vs **"Session MP3"** vs **"Session audio part"**: **Session audio** is the umbrella term for a session's persisted audio. A session is composed of one or more **Session audio parts** (one row per resume in `session_audio_parts`); each part is exactly one file — a **Session WAV** *or* a **Session MP3**, never both — at the user's chosen **Audio export format** at the time that part finalized. Different parts can have different formats if the user changed `audioExportFormat` between resumes. Don't say "the session's audio file" generically — say **Session audio part** when you mean one file, **Session audio** when you mean the whole concatenated thing, and **Session WAV** / **Session MP3** when the format matters.
- **"Hotkey"** vs **Shortcut** vs **Global hotkey** vs **Binding**: a **Shortcut** is a named in-app action; a **Binding** is the key combination assigned to it; a **Global hotkey** is a binding registered with the OS so it fires while YapStack is unfocused (used for **Dictation slots**). Don't use "hotkey" alone — qualify it.
- **"Output"** in dictation context means **Output action** (`paste`/`clipboard`/`new-note`), not the audio output device. Audio output as a *capture target* is **System audio**.
- **"Provider"** has two senses we keep distinct: the **AI provider** (OpenAI/OpenRouter/custom) for chat, and the ORT **Execution provider** (cpu/coreml/webgpu) for the Parakeet engine. Always qualify which one you mean.
- **"Provider"** vs **Connection**: an **AI provider** is a *kind* (OpenAI, OpenRouter, custom); a **Connection** is a *configured instance* of that kind. A user can have two **Connections** of the same AI provider (e.g. "Work OpenAI" and "Personal OpenAI"). Don't say "add a new provider" when the user is adding a **Connection** — the providers are a fixed enum and don't grow at runtime.
- **"Profile"** vs **Dictation slot**: both are user-named entities but they live in different layers. A **Profile** is an AI-routing object — `(Connection, Model)` — that **Feature consumers** bind to. A **Dictation slot** is a hotkey-bound capture preset that *consumes* a Profile (its cleanup Assignment). A slot is not a profile; a profile is not a slot.
- **"Slot"** is reserved for **Dictation slot**. The early design framing "provider slot" was rejected for exactly this collision; the canonical term for a configured provider instance is **Connection**.
- **"Provider configuration"**, **"endpoint"**, **"account"** are all rejected synonyms for **Connection**. Use **Connection** in all UI copy, code identifiers, docs, and PRDs.
- **"Model catalog"** is legacy terminology. There is no static catalog; a Connection's selectable models are its **Available models**, sourced from `/v1/models` at runtime and run through the **Model filter** for known kinds.
- **"aiEnabled"** is dead. Dictation slots historically carried a separate boolean for whether AI cleanup ran; that's now collapsed into the slot's **Profile Assignment** (`null` = AI off, non-null = AI on with that Profile).
- **"Active provider"** is legacy. The pre-refactor `AISettings.activeProvider` flag is gone; per-feature **Assignment** replaces it. There is no global "current provider" state anymore.
- **"Fallback"** is not a YapStack concept for AI routing. If a Connection fails, the error surfaces and the user fixes it — there is no automatic retry against a backup Profile or Connection. Don't propose fallback behavior in plans or PRDs.
- **"Engine phase"** vs **"Live phase"**: the **Engine phase** describes whether the *engine's model* is loaded and ready (idle → downloading → initializing → ready); the **Live phase** describes whether a *live transcription stream* is currently running. They're independent — the engine can be `ready` with no live stream active.
- **"Folder contains notes"** — false. **Folders** contain **Sessions** (many-to-many via `session_folders`); a session has at most one **Note**, and that note is reachable only through its session. There is no `note_folders` table.
- **"Chat is per-session"** — incomplete. A **Chat** is scoped by a **Context key** that can be a session id but can also be `global`, `pinned`, `dictation`, or `folder:{id}`. Don't say "the session's chat" if you actually mean any chat keyed off something other than a session.
- **"Sortformer is a NVIDIA model"** — true upstream, but YapStack pulls weights from the `altunenes/parakeet-rs` redistribution, not directly from NVIDIA. Don't assert NVIDIA-as-source in domain prose.
- **"Share"** is currently a **dormant concept** — the `shares` table is defined (folder-scoped) but no app code reads or writes it. Treat it as planned future work, not a live feature, until that changes.
