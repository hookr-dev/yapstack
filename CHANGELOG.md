# Changelog

All notable changes to YapStack will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed
- Backfill no longer starves live transcription. The scheduler now gates `Backfill` jobs while the live loop is busy, time-slices historical chunks before submission, and live overrun handling drains oldest contiguous slices instead of dropping the head of an overlong extraction.
- Stopping a live session is now a bounded audio endpoint. `stop_live_transcription` snapshots ring-buffer positions plus a short tail grace when invoked, the loop stops normal ingestion/restarts immediately, final live transcription and WAV flushing are capped to that boundary, and later samples are ignored for the stopped session.
- Live status now reports preserved-drain backlog instead of the removed head-drop cap counter, so slow-sidecar catch-up is visible without implying audio was discarded.
- Session WAV creation now chooses the sample rate from the configured capture source (`MicOnly`, `SystemOnly`, or mic-rate `Mixed`) instead of always preferring a stale mic buffer.
- Live-transcription lag metric in `get_live_transcription_status` and the session-end summary no longer overstates lag when backfill is interleaved with live. The underlying counter now tracks the *max* completed audio offset rather than overwriting on every chunk, so a late backfill chunk for older audio can't clobber the counter backwards and inflate the reported lag by the live/backfill offset gap.
- Session MP3 export now retains the input WAV's sample rate. `convert_wav_to_mp3` previously left LAME's output rate unset, which caused silent auto-downsampling at low bitrates (e.g. a 48 kHz input at the default 64 kbps was emitted as 22.05 kHz) — the file ended up at one rate while the `session_audio_parts.sample_rate` row stored another. Output rate is now pinned to the input rate so the DB and file always agree. New regression test `test_mp3_output_sample_rate_matches_input_at_low_bitrate` guards this.
- "Backfill in progress" UI affordance now clears immediately when the session starts with no effective backfill (resume, empty ring buffer, or requested backfill clamped to 0). Previously the FE set `backfillActive` from the *requested* value and the backend only emitted `backfill-complete` when a submitter task ran, so a user-requested backfill that the buffer couldn't honor left the affordance stuck for the whole session. Frontend now also pre-sets `backfillActive` *before* awaiting `startLiveTranscription` so a fast `backfill-complete` event arriving during the await can't race the post-await state set.
- `TranscriptionScheduler::shutdown_and_return` now aborts the worker on timeout instead of just dropping the `JoinHandle` (which only detaches it). It also no longer hands the transcription client back to shared state in that case — an aborted worker may still hold an in-flight `Arc<TranscriptionClient>` clone, so handing the same client to a new session would race the sidecar's response routing. The cleanup path emits a new `transcription-engine-dropped` event in that case; the frontend listens, resets `enginePhase` to `idle`, and re-runs `autoSetup` so the engine is ready before the next user action.

### Added
- `LiveSegmentEvent` and `LiveTranscriptionPressureEvent` carry `origin: "live" | "backfill" | "final_flush"` set by the scheduler at emit time.
- `transcription-engine-dropped` event (no payload) fired when the live-transcription cleanup path runs and the scheduler had to drop the transcription client instead of returning it (worker shutdown timeout). Frontend treats this as "engine needs re-init" and reruns `autoSetup`.
- `AGENTS.md` as canonical AI-agent instruction file (cross-tool standard); `CLAUDE.md`, `.github/copilot-instructions.md`, `.cursor/rules/main.mdc` are stubs that point to it.
- `docs/INDEX.md` (doc router), `docs/GLOSSARY.md` (domain terms), `docs/AGENT_GUIDE.md` (navigation + task recipes), `docs/LINEAR_TICKETS.md` (agent-pickup ticket structure).
- `docs/adr/` directory with ADR-0001 (adopt AGENTS.md).
- `.github/ISSUE_TEMPLATE/agent_ready_task.yml` for AI-agent-pickup tickets.
- CONTRIBUTING.md sections: Quickstart, Where to start, AI-Assisted Contributions, Definition of Done, Verification commands, Scope boundaries.
- AGENTS.md "Permission boundaries" section (Always / Ask first / Never) and a pre-commit checklist.

### Removed
- `is_backfill: boolean` field on `LiveSegmentEvent` and `LiveTranscriptionPressureEvent`. Use `origin` instead — it's a strict superset (distinguishes `live` from `final_flush`, where `is_backfill` could not).

### Changed
- CONTRIBUTING.md restructured for both human and AI contributors.

## [1.0.0-alpha.5] - 2026-04-28

First public alpha release.

### Added

- Real-time mic + system audio capture with always-on 5-minute ring buffer.
- Whisper transcription engine (Metal-accelerated on Apple Silicon, broad language support).
- Parakeet TDT v3 transcription engine via parakeet-rs + ONNX Runtime, with int8 variant + WebGPU acceleration on Apple Silicon.
- Sortformer speaker diarization (Parakeet only).
- Per-source VAD (Silero V5), hallucination filtering, two-tier prompt context, prompt decay.
- Stream health monitoring with auto-restart (up to 3 attempts).
- Voice dictation: multiple slots, hold/toggle modes, per-slot AI processing prompts, paste/copy/note actions, history grouped by day.
- Per-session AI chat with multi-turn tool calling (10 tools: rename, pin, save-to-notes, tag, folder, search, etc.).
- Tiptap rich-text notes editor with version history.
- Folders + tags + Cmd-K search across sessions, notes, and segments.
- Privacy-first analytics via Aptabase (no transcript content tracked).
- macOS desktop integration: system tray, recording indicator overlay, customizable global shortcuts.
- Custom audio playback protocol with seeking and 0.5×–2× playback speeds.

### Platform support

- macOS (Apple Silicon): officially supported.
- macOS (Intel): best-effort.
- Windows: experimental — local builds only, no official CI/CD artifacts.
- Linux: not yet supported.
