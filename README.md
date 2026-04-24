<p align="center">
  <img src="apps/desktop/src-tauri/icons/icon.png" alt="YapStack" width="128" height="128" />
</p>

<h1 align="center">YapStack</h1>

<p align="center">
  Real-time audio capture & transcription for your desktop.
</p>

---

YapStack is a privacy-first desktop app that captures mic and system audio, transcribes it locally using [Whisper](https://github.com/ggerganov/whisper.cpp), and organizes everything into searchable, editable notes. All processing happens on-device — nothing leaves your machine.

<!-- TODO: Add screenshot/demo GIF here -->

## Features

### Never Miss a Word

Always-on ring buffer (up to 5 min) captures audio before you hit record. Start a session and rewind to grab what you missed. Backfill transcribes retroactively while live transcription continues in parallel.

### Real-Time Transcription

Local Whisper inference with per-source VAD, hallucination filtering, and prompt context for continuity. All on-device.

### Full Audio Capture

Sessions stream to WAV incrementally. Play back at 6 speeds (0.5×–2×) with seeking. Click any transcript timestamp to jump to that moment.

### Voice Dictation

Global shortcut-driven dictation with deep customization:

- **Multiple slots** — Named slots with custom global keybinds
- **Activation modes** — Hold-to-talk or toggle
- **AI processing** — Per-slot system prompts to transform speech (e.g. "Clean & Focus", "Create Spec")
- **Output actions** — Paste into active app, copy to clipboard, or create a new note
- **Status bubble** — Floating overlay (listening → transcribing → processing → done)
- **History** — Past dictations grouped by day with audio replay

### AI Session Chat

Per-session chat with tool calling: rename, pin, save to notes — each with 10s undo. AI cites transcript segments as clickable timestamp chips.

### Rich Notes Editor

Tiptap split-pane editor alongside the transcript. Version history with restore.

### Mic + System Audio

Capture mic, system audio, or both. Independent per-source VAD. Stream health monitoring with auto-restart (up to 3 attempts).

### Organization & Search

Folders with icons and colors, pinning, drag-and-drop sorting, Cmd+K search across sessions, notes, and segments.

### Desktop Integration

System tray with quick actions. Fully customizable global shortcuts. Recording indicator overlay. Close-to-minimize. macOS + Windows.

## Getting Started

### Prerequisites

- **Rust** ≥ 1.77.2 — install via [rustup](https://rustup.rs)
- **Node.js** ≥ 22
- **pnpm** — `corepack enable && corepack prepare pnpm@latest-10 --activate`
- **cmake** — required only if building with the `whisper` feature flag

### Setup

```bash
pnpm install
```

### Development

```bash
# Full app (Tauri + Vite)
pnpm tauri dev

# Frontend only
pnpm --filter @yapstack/desktop dev
```

### Build

```bash
pnpm tauri build
```

### Testing

```bash
# Everything
pnpm check          # Rust build + test + fmt + clippy + TS typecheck + ESLint + vitest

# Selective
pnpm test           # Rust + frontend tests
pnpm test:frontend  # Vitest only
pnpm test:rust      # cargo test --all
pnpm lint           # Rust fmt + clippy + ESLint
pnpm typecheck      # TypeScript type checking
```

## Architecture

Tauri v2 app with a Rust backend and React frontend. Five crates handle distinct concerns:

| Crate | Role |
|-------|------|
| `yapstack-common` | Shared types, config, audio utilities |
| `yapstack-audio` | Lock-free ring buffers, mic/system capture, WAV export |
| `yapstack-transcription` | Model management, sidecar IPC |
| `yapstack-sidecar` | Standalone Whisper inference binary |
| `yapstack-desktop` | Tauri command layer, live transcription controller |

Detailed docs under [`docs/`](docs/):

- [`ARCHITECTURE.md`](docs/ARCHITECTURE.md) — data flow, crates, IPC, state, analytics.
- [`API_REFERENCE.md`](docs/API_REFERENCE.md) — Rust library + Tauri command signatures.
- [`DEVELOPMENT.md`](docs/DEVELOPMENT.md) — build, feature flags, test setup.
- [`FRONTEND.md`](docs/FRONTEND.md) — Tailwind tokens, shadcn inventory, framework stack, shortcuts, UX patterns.
- [`AI_CONTEXT.md`](docs/AI_CONTEXT.md) — AI chat context, tool registry + how to add a tool, folders, pending tags schema.
- [`PRINCIPLES.md`](docs/PRINCIPLES.md) — design, testing, and coding posture.
- [`LOCAL_LLM.md`](docs/LOCAL_LLM.md) — llama.cpp, LM Studio, Ollama integration.
- [`IMPLEMENTATION_LOG.md`](docs/IMPLEMENTATION_LOG.md) — phase-by-phase build history.

## Tech Stack

- [Tauri v2](https://v2.tauri.app) — native desktop shell
- [Rust](https://www.rust-lang.org) — audio capture, transcription orchestration
- [React 19](https://react.dev) + TypeScript — frontend UI
- [SQLite](https://sqlite.org) — session, note, and folder persistence
- [Whisper.cpp](https://github.com/ggerganov/whisper.cpp) — on-device speech recognition
- [Tiptap](https://tiptap.dev) — rich text editor
