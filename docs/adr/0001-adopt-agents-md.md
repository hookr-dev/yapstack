# 0001. Adopt AGENTS.md as the canonical agent instruction file

- **Status**: Accepted
- **Date**: 2026-04-28

## Context

YapStack is developed with AI coding assistants as a first-class part of the workflow. Multiple agent tools each look for their own instruction file: Claude Code reads `CLAUDE.md`, Codex reads `AGENTS.md`, GitHub Copilot reads `.github/copilot-instructions.md`, Cursor reads `.cursor/rules/*.mdc`. Maintaining duplicates of the same content across these files leads to drift.

In December 2025, OpenAI donated the `AGENTS.md` specification to the Linux Foundation under the Agentic AI Foundation. As of early 2026, ~25 agent tools (including all of the above) read `AGENTS.md` either as primary or fallback. The cross-tool standard is settling.

## Decision

Adopt `AGENTS.md` at the repo root as the canonical instruction file. All other agent-instruction files become **stubs that point to it**.

Files affected:
- `AGENTS.md` — full content (build commands, permission boundaries, conventions, changelog discipline).
- `CLAUDE.md` — stub: "see AGENTS.md".
- `.github/copilot-instructions.md` — stub: "see AGENTS.md".
- `.cursor/rules/main.mdc` — stub with `alwaysApply: true` pointing to AGENTS.md.

Nested `AGENTS.md` files are allowed when subsystem-specific rules genuinely differ (e.g., a future `crates/yapstack-transcription-sidecar/AGENTS.md` covering the feature-flag matrix). They override the root for files within their scope per the agents.md spec.

## Consequences

- **Single source of truth**: agent guidance edits land in one file. No drift between Claude/Codex/Copilot/Cursor.
- **Discoverability**: humans curious about how AI agents are configured find one file at the obvious location.
- **Cross-tool support**: any future agent that follows the agents.md spec will pick up our rules without configuration.
- **Trade-off accepted**: tool-specific shortcuts (e.g., Cursor's `.mdc` glob targeting) are not used. Worth it for uniformity.
- **Anti-pattern avoided**: per [Augment Code's empirical study](https://www.augmentcode.com/guides/how-to-build-agents-md), LLM-auto-generated AGENTS.md files reduce task success and inflate token cost. Ours is hand-written and trimmed; we delete unhelpful rules instead of accumulating them.

## References

- [agents.md spec](https://agents.md/)
- [OpenAI Codex AGENTS.md guide](https://developers.openai.com/codex/guides/agents-md)
- [Augment Code: How to Build AGENTS.md](https://www.augmentcode.com/guides/how-to-build-agents-md)
