# Documentation index

Fast router for humans and AI agents. One line per doc — load this file first when orienting.

## Top-level

- [`README.md`](../README.md) — project overview, install, platform support, license.
- [`AGENTS.md`](../AGENTS.md) — canonical AI agent instructions (build/test commands, permission boundaries, conventions).
- [`CONTRIBUTING.md`](../CONTRIBUTING.md) — contributor workflow for humans and agents.
- [`CODE_OF_CONDUCT.md`](../CODE_OF_CONDUCT.md) — Contributor Covenant 2.1.
- [`SECURITY.md`](../SECURITY.md) — vulnerability disclosure policy.
- [`CHANGELOG.md`](../CHANGELOG.md) — release notes (Keep a Changelog format).

## Architecture & API

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — data flow between crates, ring buffer, sidecar IPC, live transcription pipeline, AI chat tool calling, frontend component tree, analytics.
- [`API_REFERENCE.md`](API_REFERENCE.md) — exact function signatures, struct fields, error variants, Tauri command shapes. Read before adding or modifying public APIs.
- [`GLOSSARY.md`](GLOSSARY.md) — domain terms (session, segment, part, dictation, diarization, etc.).

## Development

- [`DEVELOPMENT.md`](DEVELOPMENT.md) — build issues, feature flags, sidecar compilation, test infra, model paths.
- [`FRONTEND.md`](FRONTEND.md) — Tailwind tokens, shadcn inventory, framework stack, shortcuts, UX patterns.
- [`PRINCIPLES.md`](PRINCIPLES.md) — design, testing, and coding posture. Read before refactoring.
- [`AGENT_GUIDE.md`](AGENT_GUIDE.md) — navigation tips and common task recipes for AI agents.
- [`LINEAR_TICKETS.md`](LINEAR_TICKETS.md) — ticket structure for agent pickup.
- [`RELEASE.md`](RELEASE.md) — release runbook: version bump locations, CHANGELOG roll, tag/push, signing, draft publish, hotfix path.

## Subsystems & integrations

- [`AI_CONTEXT.md`](AI_CONTEXT.md) — AI chat context flow, tool registry + how to add a tool.
- [`LOCAL_LLM.md`](LOCAL_LLM.md) — llama.cpp, LM Studio, Ollama integration.

## History & decisions

- [`IMPLEMENTATION_LOG.md`](IMPLEMENTATION_LOG.md) — phase-by-phase build history. Use to understand *why* something was built a certain way.
- [`adr/`](adr/) — architecture decision records (append-only).

## Plans (transient)

- [`plans/`](plans/) — historical implementation plans, mostly archived. Browse only when researching prior approaches.
