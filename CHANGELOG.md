# Changelog

All notable changes to YapStack will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.0.0-alpha.8] - 2026-05-02

### Added
- **Dictation volume ducking (macOS).** Opt-in setting on the Dictation tab lowers the system output volume to a configured target (default 20%) while a dictation is recording, then restores it the moment recording ends. Only ever lowers — never raises — so a quieter starting volume isn't bumped up. Tracks the *device* you started ducking, so swapping outputs mid-dictation (AirPods connect, USB DAC unplug) restores the original device rather than leaving it stuck at the ducked level. No-op on Windows / Linux (#22).

### Changed
- **Backfill no longer starves live transcription.** Long backfill jobs are now sliced and yield to live work at the sidecar. Live audio keeps up with real time even when a fresh session is rewinding into a full 5-minute backfill window (#20).
- **Stopping a live session is now a clean cut-off.** When you press stop, the loop snapshots the audio boundary at that instant plus a short tail grace, finalizes whatever's inside, and ignores anything captured after. The final transcript and WAV no longer drift based on host load (#20).
- **Session WAV uses the right sample rate** for the configured capture source (`MicOnly`, `SystemOnly`, or `Mixed`) rather than always preferring whichever buffer happened to exist (#20).
- **Live-status diagnostics:** the slow-sidecar indicator now reports preserved-drain backlog (audio queued behind inference) instead of the old head-drop counter, which implied audio was being discarded. Audio is now preserved through catch-up; the new fields show how far behind the live tier is (#20).
- **Repository now follows the `AGENTS.md` convention** for AI-assistant instructions. `CLAUDE.md`, `.github/copilot-instructions.md`, and `.cursor/rules/main.mdc` are now thin stubs pointing at the canonical `AGENTS.md`. New documentation routing under `docs/INDEX.md`, `docs/GLOSSARY.md`, `docs/AGENT_GUIDE.md`, `docs/LINEAR_TICKETS.md`, and `docs/adr/`. CONTRIBUTING.md restructured for both human and AI contributors with Quickstart, Definition of Done, and Verification commands sections.
- **Internal sidecar crate renamed** `yapstack-sidecar` → `yapstack-transcription-sidecar` to make room for additional sidecar workers (e.g. embeddings) without naming ambiguity. No user-visible behaviour change; build scripts are now `scripts/build-sidecars.sh` (wrapper) and `scripts/build-transcription-sidecar.sh` (#21).

### Fixed
- **Session MP3 export no longer silently downsamples.** A 48 kHz capture exported at 64 kbps is now written at 48 kHz; previously LAME's auto-rate selection dropped it to 22.05 kHz, leaving the audio file and the database disagreeing on sample rate (#20).
- **"Processing prior audio" divider** in the chat now sits at the actual backfill→live boundary instead of being pinned to the bottom of the transcript. As the rewind buffer streams in, the divider slides up into the right place (#19).
- **Lag metric is no longer overstated** when backfill chunks arrive after live chunks. The reported lag tracks the maximum completed audio offset rather than overwriting on every chunk, so a late backfill chunk for older audio can't pull the counter backwards and inflate the displayed lag (#20).
- **"Backfill in progress" affordance clears immediately** when there's nothing to backfill (resume, empty buffer, or a backfill request that the ring buffer can't honor). Previously the badge could stay stuck for the entire session (#20).
- **Transcription engine self-heals after a wedged shutdown.** If the engine fails to release cleanly when stopping a session (rare; happens when the sidecar hangs past the 5-minute drain ceiling), the app now resets engine state and re-runs auto-setup before the next action instead of leaving the next session to fail with `NotInitialized` (#20).
- **Dev sidecar mirror now survives `cargo` rebuilds.** Local development could leave the feature-rich sidecar binary clobbered by a feature-poor `cargo build` rebuild, breaking subsequent `pnpm tauri dev` runs (#18).

## [1.0.0-alpha.7] - 2026-04-30

### Added
- Folder auto-recommendation: when keywords from a folder's name, description, or tags on sessions previously filed under it appear in the live transcript, the top folder surfaces inline above the session view as a "Recommended" pill. Click the pill to confirm "Add to {folder}", pick a different folder from the full list, or dismiss. Single-folder workspaces show inline accept/dismiss instead of a dropdown (#17).
- Default dictation slots for new users: "Clean & Focus" (Ctrl+Shift+C) and "Engineer" (Ctrl+Shift+X) AI post-processors alongside Raw Dictation (now bound to Ctrl+Shift+D). Existing users' configured slots are untouched (#17).
- Mid-session folder picker: the session header actions dropdown renders during recording with a Folders submenu, so a session can be filed without stopping it (#17).
- "Show audio file" action on dictation history entries (icon button + context-menu item) — reveals the WAV/MP3 in Finder, matching the existing session affordance (#17).
- Mic-only clarification in the Dictation settings header so users know dictation never captures system audio (#17).

### Changed
- Rewind dropdown supports short buffers: the "Full buffer" action now honors any available backfill — including sub-30-second and sub-second amounts — rather than gating on hardcoded steps. Sub-second precision is preserved end-to-end (#16).
- Engine and capture status now surface only via the title-bar status dot. The centered "Loading transcription engine…" banner in the main content area is gone. The dot pulses amber during downloading/initializing and is steady green only when actually capturing (#17).
- Mid-session context menu replaces the disabled "Delete" with a destructive "Stop recording" entry so users have an actionable path forward instead of a silently disabled item (#17).
- Rebinding a shortcut to a key already used by another shortcut (or dictation slot) now rejects with a toast naming the conflicting owner. Previously the new binding silently stole the key from its prior owner (#17).
- Updater install progress bar is strictly monotonic. Repeated `Started` events and out-of-order chunk callbacks no longer pull the bar backward; sub-1% chunks no longer trigger redundant re-renders. The same guard applies to the model-download progress (Whisper / Parakeet / Sortformer fetch) (#17).

### Fixed
- Auto-suggested folder no longer keeps surfacing other folders after the user has already filed the session. Acceptance and override are now one-shot for the session; dismiss stays per-folder so the next-best match can still surface if the user keeps rejecting picks (#17).

## [1.0.0-alpha.6] - 2026-04-30

### Fixed
- Voice dictation: random/garbled characters when copying transcripts via `pbcopy` on macOS — `LC_CTYPE` is now forced to `UTF-8` when invoking `pbcopy` so multibyte output round-trips correctly (#15).
- CI: install `cmake` before the Rust build so whisper.cpp configures cleanly.
- CI: pass `-march=native` so whisper.cpp detects `i8mm` on Apple Silicon runners.
- CI: stub `libwebgpu_dawn.dylib` placeholder alongside the existing ONNX Runtime placeholders.

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
