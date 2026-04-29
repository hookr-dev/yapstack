# Changelog

All notable changes to YapStack will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed
- Backfill audio is no longer silently dropped when a session is stopped. A new `TranscriptionScheduler` (priority queue: FinalFlush > Live > Backfill, with mic/system round-robin at the live tier) sits in front of the sidecar, so live work preempts backfill at the engine instead of starving it, and the backfill submitter is allowed to drain on stop instead of being cancelled at the next chunk boundary. Closing-words chunks are submitted at FinalFlush priority so they outrank pending backfill and survive the stop path.

### Added
- `LiveSegmentEvent` carries `origin: "live" | "backfill" | "final_flush"` and a monotonic `event_sequence` counter; legacy `is_backfill: bool` retained for backwards compat.
- `AGENTS.md` as canonical AI-agent instruction file (cross-tool standard); `CLAUDE.md`, `.github/copilot-instructions.md`, `.cursor/rules/main.mdc` are stubs that point to it.
- `docs/INDEX.md` (doc router), `docs/GLOSSARY.md` (domain terms), `docs/AGENT_GUIDE.md` (navigation + task recipes), `docs/LINEAR_TICKETS.md` (agent-pickup ticket structure).
- `docs/adr/` directory with ADR-0001 (adopt AGENTS.md).
- `.github/ISSUE_TEMPLATE/agent_ready_task.yml` for AI-agent-pickup tickets.
- CONTRIBUTING.md sections: Quickstart, Where to start, AI-Assisted Contributions, Definition of Done, Verification commands, Scope boundaries.
- AGENTS.md "Permission boundaries" section (Always / Ask first / Never) and a pre-commit checklist.

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
