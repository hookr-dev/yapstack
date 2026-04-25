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
| **Stream restart**  | Recovering a failed cpal stream in place, reusing the existing ring buffer so no audio is lost.                           | Reconnect, reset                  |
| **Stream health**   | The frontend-visible status of stream supervision: `restarted`, `restart_failed`, `restart_abandoned`.                    | Stream state                      |

## Sessions and segments

| Term                  | Definition                                                                                                            | Aliases to avoid          |
| --------------------- | --------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| **Session**           | A bounded recording with its captured audio and ordered list of segments, persisted in SQLite.                        | Recording, take           |
| **Segment**           | A single transcribed utterance with start/end timestamps, text, confidence, and optional speaker ID.                  | Chunk, transcript line    |
| **Backfill**          | Re-transcription of audio captured *before* live transcription started, emitted as `is_backfill: true` segments.      | History, replay, prefill  |
| **Session audio**       | The persisted on-disk audio artifact for a session, served via the `audio-stream://` protocol. Encoded as either WAV *or* MP3 — never both — per the user's audio export format. | Audio file, recording file |
| **Session WAV**         | The 16-bit PCM mono WAV form of a session's audio. Always streamed to `$APP_DATA_DIR/audio/{session_id}.wav` during recording; survives finalization only when audio export format is `wav`. | Wav file                   |
| **Session MP3**         | The MP3 form of a session's audio, produced at finalization by encoding the streamed WAV and **deleting** the WAV. The session keeps only the `.mp3`. | Compressed audio           |
| **Audio export format** | The user's choice of persisted session-audio encoding: `wav` or `mp3`. Applied at session finalization (and to user-triggered re-saves).             | Output format, save format |
| **MP3 bitrate**         | The user-configurable encode quality for the MP3 form (8–320 kbps).                                                                                  | Quality, kbps              |
| **Audio save location** | The filesystem path used when the user saves a copy of session audio outside the app data directory.                                                | Export path                |

## Transcription engines

| Term                | Definition                                                                                                          | Aliases to avoid                |
| ------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| **Engine**          | A transcription backend the sidecar can run: `Whisper` or `Parakeet`. Selected per session.                         | Backend, model type, provider   |
| **Variant**         | A specific weights bundle for an engine, e.g. a Parakeet TDT v3 directory.                                          | Model size, flavor              |
| **Model**           | The downloaded weights on disk: a single ggml file (Whisper) or a multi-file ONNX directory (Parakeet, Sortformer). | Weights, checkpoint             |
| **Sidecar**         | The standalone `yapstack-sidecar` process that hosts the chosen engine and speaks JSON-line IPC.                    | Worker, helper process          |
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
| **Dictation**              | A short, sessionless voice-to-text capture optimized for dropping text into the active app or clipboard.                | Quick note, voice command |
| **Dictation slot**         | A named, reusable dictation preset. Carries id, name, enabled flag, AI-enabled flag, prompt, output action, and default binding. The slots array is unlimited; one slot ("Raw Dictation") ships by default. | Preset, profile           |
| **Dictation activation mode** | How a slot's hotkey behaves: `hold` (push-to-talk, recording while held) or `toggle` (press to start/stop).          | Trigger mode              |
| **Output action**          | What to do with a finished dictation: `paste` into the focused field, `clipboard`, or `new-note` (create a session).    | Insertion mode, sink      |
| **Dictation history**      | Persisted log of past dictation outputs, distinct from sessions and exposed in the sidebar's `dictation` list filter.   | Recents                   |

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

## AI chat

| Term              | Definition                                                                                                                          | Aliases to avoid       |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| **Chat**          | A conversation with an LLM scoped to a context (a session, a folder, the pinned set, dictation history, or global), exposed via the floating chat bar. | Assistant, copilot     |
| **Context key**   | The string that scopes a chat: `global`, `pinned`, `dictation`, `folder:{id}`, or a session id. Determines what content the LLM sees and where messages are filed. | Scope, channel         |
| **Chat message**  | One turn (user or assistant) in a chat, persisted in SQLite with its context key.                                                   | Reply, exchange        |
| **Tool**          | A typed function the LLM can invoke (`update_title`, `save_to_notes`, `pin_session`) via OpenAI tool calling.                       | Action, function call  |
| **Citation**      | An inline `[[seg:ID]]` reference in chat output that resolves to a clickable timestamp chip on a segment.                           | Reference, link        |
| **Undo window**   | The 10-second period after a tool mutation during which the user can revert it.                                                     | Grace period, rollback |
| **AI provider**   | An external LLM backend the chat can target: `openai`, `openrouter`, or `custom` (any OpenAI-compatible endpoint).                  | LLM provider, vendor   |
| **API key**       | A user-supplied secret authenticating chat requests to a provider. Stored locally; BYO-key.                                         | Token, credential      |
| **Model catalog** | The per-provider list of selectable chat models — built-in for known providers, fetched at runtime for custom.                      | Model list             |

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

- A **Session** owns one **Session audio** artifact (a **Session WAV** *or* a **Session MP3**, never both) and many **Segments**. The format is decided at finalization by the **Audio export format** setting; choosing `mp3` re-encodes the streamed WAV and deletes it.
- A **Session** has zero or one **Note** (enforced by `UNIQUE` on `notes.session_id`); a **Note** has zero or many **Note versions**.
- A **Session** can belong to zero or many **Folders**, and a **Folder** can hold zero or many **Sessions** — many-to-many via the `session_folders` join table. **Folders do not contain Notes directly**; a note is reachable through its session.
- A **Capture** writes into one **Ring buffer** per **Source**; **Live transcription** reads from those buffers.
- An **Engine** runs inside the **Sidecar**; the **Transcription client** is its in-process handle. The engine's readiness is tracked as the **Engine phase**.
- **Diarization** assigns a **Speaker ID** to a **Segment**; the user maps it to a **Speaker label** per-session (Zustand-persisted, not in SQL).
- A **Chat** is scoped by a **Context key** (a session id, a folder id, `pinned`, `dictation`, or `global`) and uses one **AI provider** + **API key** + selected model from the **Model catalog**. Session-scoped chats may emit **Citations** that resolve to that session's **Segments**.
- **Dictation** produces transcribed text without creating a **Session** unless the active **Dictation slot**'s **Output action** is `new-note`. Either way it is logged in **Dictation history** (with an optional session FK).
- A **Dictation slot** owns one **Binding** (a **Global hotkey**) and one **Dictation activation mode**.
- **Capture source** = `Mixed` is the only mode that consults **Mix config** (mic gain, system gain, normalize).

## Example dialogue

> **Dev:** "When **live transcription** runs with **Mixed** capture, do mic and system share **VAD** state?"

> **Domain expert:** "No. Each **source** has its own Silero stream state and its own **chunk** cursor. They share the Silero ONNX session, but a mic utterance ending doesn't close a system utterance."

> **Dev:** "And the **initial prompt** — is that per-source too?"

> **Domain expert:** "Per-source on Whisper, yes — each source carries its own **prompt context**. On Parakeet the engine has no text prompt at all, so we drop it. **Prompt decay** still runs on both engines so we don't preserve stale state across long silences."

> **Dev:** "If **diarization** is on, when does the **speaker ID** show up on a **segment**?"

> **Domain expert:** "Only with the **Parakeet** engine. The **sidecar** runs **Sortformer** on the same chunk audio, picks the max-overlap speaker per segment, and returns it inline. The frontend then groups consecutive same-ID segments under a **speaker label** the user can rename."

> **Dev:** "And **backfill** segments — do they go through the same path?"

> **Domain expert:** "Same transcribe call, just flagged `is_backfill: true`. Backfill seeds the prompt context once at start; after that, **prompt decay** can clear it like any other run."

> **Dev:** "When a **Session** finalizes, what determines whether we end up with a WAV or an MP3?"

> **Domain expert:** "The **Audio export format** setting. We always stream a **Session WAV** to disk during recording — that's the working buffer. At finalization, if the format is `wav`, we keep it. If it's `mp3`, we encode at the configured **MP3 bitrate** to produce a **Session MP3** and delete the WAV. The session ends up with exactly one **Session audio** artifact."

> **Dev:** "And **Dictation** — does that produce a session audio artifact too?"

> **Domain expert:** "Only if the slot's **Output action** is `new-note`. Otherwise the audio just feeds transcription, the text goes to **Output action** `paste` or `clipboard`, and the transcript lands in **Dictation history** — no session, no audio kept."

> **Dev:** "If I drag a **Session** into a **Folder**, does it leave its current folder?"

> **Domain expert:** "No — a session can sit in many folders at once. The relationship is many-to-many through the `session_folders` join table. Adding to a new folder doesn't remove it from any existing one; you have to remove it explicitly."

> **Dev:** "And the **Note** for that session — does it move with the folder, or have its own folder?"

> **Domain expert:** "A **Note** has no folder of its own. It's pinned 1:1 to a single **Session** by `notes.session_id`, and you reach it through the session. Folders contain sessions; notes ride along."

## Flagged ambiguities

- **"Whisper client"** (legacy) vs **Transcription client** (current). The Tauri state name `WhisperClientState` and the type alias `WhisperClient` still exist for one-release back-compat, but they hold an engine-agnostic `TranscriptionClient`. Prefer **Transcription client** in all new prose, comments, and PR titles.
- **"Segment"** is overloaded: it means both a transcribed unit (the domain segment, persisted in SQLite) and, internally, a `WhisperSegment` from the whisper-rs API. In domain language, **Segment** always means the persisted transcribed unit. Use "whisper-rs segment" or "raw segment" when referring to the library type.
- **"Chunk"** vs **Segment**: a **chunk** is the *audio slice* sent to the sidecar between VAD start/end; a **segment** is what comes back transcribed. Don't use "chunk" to mean transcribed text.
- **"Source"** has two distinct meanings: (1) a **Capture source** enum (`MicOnly`/`SystemOnly`/`Mixed`) chosen at session start, and (2) a single audio origin (`mic` or `system`) within the live loop. The first is a *configuration*, the second is a *runtime entity* with its own VAD and cursor. Prefer **Capture source** for the enum and **Source** alone only inside the live-transcription context.
- **"Model"** vs **Variant** vs **Engine**: an **Engine** is the algorithm family (Whisper/Parakeet); a **Variant** is a specific weights bundle for that engine; a **Model** is the on-disk artifact. "Switching models" is ambiguous — say "switching engine" or "switching variant" depending on intent.
- **"Speaker name"** vs **Speaker label**: code uses `speakerNames` (the persisted Zustand map). In domain prose prefer **Speaker label** to make clear it's a user-facing display string mapped from a numeric **Speaker ID**.
- **"Recording"** is used loosely in UI copy. In domain language a **Session** is the persisted record and a **Capture** is the live act. Avoid "recording" as a noun unless quoting UI text.
- **"Session WAV"** vs **"Session audio"** vs **"Session MP3"**: **Session audio** is the umbrella term for a session's persisted audio artifact, and a session has **exactly one** — either a **Session WAV** *or* a **Session MP3**, never both. The streamed WAV is always the working form *during* recording; at finalization the **Audio export format** setting decides which encoding survives, and choosing `mp3` deletes the WAV. Don't say "the session's audio file" generically when the format matters — say **Session WAV** or **Session MP3**.
- **"Hotkey"** vs **Shortcut** vs **Global hotkey** vs **Binding**: a **Shortcut** is a named in-app action; a **Binding** is the key combination assigned to it; a **Global hotkey** is a binding registered with the OS so it fires while YapStack is unfocused (used for **Dictation slots**). Don't use "hotkey" alone — qualify it.
- **"Output"** in dictation context means **Output action** (`paste`/`clipboard`/`new-note`), not the audio output device. Audio output as a *capture target* is **System audio**.
- **"Provider"** has two senses we keep distinct: the **AI provider** (OpenAI/OpenRouter/custom) for chat, and the ORT **Execution provider** (cpu/coreml/webgpu) for the Parakeet engine. Always qualify which one you mean.
- **"Engine phase"** vs **"Live phase"**: the **Engine phase** describes whether the *engine's model* is loaded and ready (idle → downloading → initializing → ready); the **Live phase** describes whether a *live transcription stream* is currently running. They're independent — the engine can be `ready` with no live stream active.
- **"Folder contains notes"** — false. **Folders** contain **Sessions** (many-to-many via `session_folders`); a session has at most one **Note**, and that note is reachable only through its session. There is no `note_folders` table.
- **"Chat is per-session"** — incomplete. A **Chat** is scoped by a **Context key** that can be a session id but can also be `global`, `pinned`, `dictation`, or `folder:{id}`. Don't say "the session's chat" if you actually mean any chat keyed off something other than a session.
- **"Sortformer is a NVIDIA model"** — true upstream, but YapStack pulls weights from the `altunenes/parakeet-rs` redistribution, not directly from NVIDIA. Don't assert NVIDIA-as-source in domain prose.
- **"Share"** is currently a **dormant concept** — the `shares` table is defined (folder-scoped) but no app code reads or writes it. Treat it as planned future work, not a live feature, until that changes.
