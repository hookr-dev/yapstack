# Architecture Decision Records

Append-only log of structural decisions and their rationale. Read these to understand *why* the codebase looks the way it does.

## Format

`NNNN-short-title.md` numbered sequentially. Each ADR has:

- **Title** — short verb phrase ("Adopt AGENTS.md...", "Use SQLite for...").
- **Status** — Proposed / Accepted / Deprecated / Superseded by NNNN.
- **Date** — ISO date adopted.
- **Context** — what problem prompted the decision.
- **Decision** — what we chose.
- **Consequences** — what changes for contributors as a result; trade-offs accepted.

## Append-only rule

Don't edit accepted ADRs. To change a decision, write a new ADR that supersedes the prior one and update the prior ADR's status to `Superseded by NNNN`.

## Index

- [`0001-adopt-agents-md.md`](0001-adopt-agents-md.md) — canonical AI-agent instruction file.
- [`0002-chat-context-settings-frontend-schema.md`](0002-chat-context-settings-frontend-schema.md) — per-chat Profile override table created via frontend runtime schema, not a numbered migration.
