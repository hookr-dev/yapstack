# Linear ticket structure

YapStack tickets are written so that **either a human or an AI agent** can pick them up and execute autonomously. Tickets that follow this structure get the `agent-ready` label and become candidates for assignment to a coding agent (Claude Code, Codex, Copilot coding agent).

## Required fields

Every issue or PR must answer these in order. Sloppy or missing answers block agent pickup.

### 1. Problem (1-3 sentences, no solution)

What user-facing or developer-facing problem is this solving? Who hits it, how often, what's the symptom? Lead with the *problem*, not the proposed fix — alternative solutions die when the solution is baked into the problem statement.

❌ *Bad*: "Add a debounce to the dictation start handler."
✅ *Good*: "Pressing the dictation hotkey twice in quick succession occasionally creates two overlapping sessions, leaving a stranded recording in the DB. Reproducible ~1 in 10 attempts on macOS."

### 2. Acceptance criteria (testable checklist)

Bulleted, each item independently verifiable. Tests, log lines, UI states — not vibes.

```
- [ ] Two hotkey presses within 200ms produce exactly one session row in the DB.
- [ ] No `recording`-status orphan rows after rapid-fire start/stop.
- [ ] New unit test in `dictation.test.ts` covering the rapid double-trigger.
- [ ] `pnpm check` passes.
```

### 3. Files likely involved

Concrete paths, not vague areas. The agent uses these as starting points, not limits.

```
- apps/desktop/src/hooks/useDictation.ts
- apps/desktop/src-tauri/src/commands/dictation.rs
- apps/desktop/src/test/tauri-mocks.ts
```

### 4. Out of scope (explicit non-goals)

The single biggest agent-failure mode is scope creep. List what is **not** part of this ticket, even when tempting.

```
- Refactoring the dictation slot config storage layer.
- Renaming the `start_dictation` command.
- Touching the recording-indicator overlay window.
```

### 5. Verification command

The exact shell command(s) the agent should run to confirm done. Most often `pnpm check`, sometimes a more targeted subset.

```
pnpm check
# or for a tight loop while iterating:
pnpm test:frontend -- useDictation
```

### 6. Definition of done (project-level, link only)

Project-wide DoD lives in [`CONTRIBUTING.md`](../CONTRIBUTING.md#definition-of-done). The ticket only needs to reference it; not repeat it.

## Optional fields

- **Reproduction steps** — for bugs.
- **Hypotheses / suspects** — if you've debugged but not fixed, list current theories with evidence.
- **Related tickets** — `Relates to #123`, `Blocks #456`. Linear handles graph automatically.
- **Risk** — anything in the [`AGENTS.md`](../AGENTS.md) "Ask first" or "Never" lists. Flag explicitly so the agent surfaces before acting.

## Labels

- `agent-ready` — meets all required fields. Eligible for agent assignment.
- `needs-triage` — opened, not yet triaged.
- `blocked` — waiting on external input.
- `needs-human` — explicitly off-limits for agents (e.g., legal review, design judgement).
- `bug`, `enhancement`, `refactor`, `docs`, `chore` — type tags.

## Workflow for agent-pickup tickets

1. Triage moves a ticket to `Ready` and applies `agent-ready` if all required fields are filled.
2. Maintainer assigns to an agent via Linear's agent integration (or pastes the ticket into a Claude Code / Codex session).
3. Agent works the ticket: branch off `main`, implement, run verification, open PR.
4. Agent's PR description repeats the Problem / AC / Verification sections from the ticket so the PR is self-contained.
5. Human reviews the PR, focuses on *judgment calls* (architecture, scope, naming) not mechanical correctness — CI gates that.
6. On merge, ticket auto-closes (Linear/GitHub link).

## Anti-patterns

- **One ticket, multiple problems.** Split. Agents (and humans) execute better on focused tickets.
- **Solution-first phrasing.** Forces a specific approach before alternatives are weighed.
- **Vague acceptance criteria** ("works correctly", "handles edge cases"). Replace with explicit cases.
- **No out-of-scope list.** Leads to scope creep and ballooned PRs.
- **No verification command.** Agent doesn't know how to declare done.
- **AC that requires reading half the codebase to understand.** Tighten the problem statement, link to docs, define terms in [`docs/GLOSSARY.md`](GLOSSARY.md) if needed.

## Example: complete agent-ready ticket

> **Title**: Debounce rapid dictation hotkey to prevent stranded sessions
>
> **Problem**: Pressing the dictation hotkey twice in quick succession (≤200ms) occasionally creates two overlapping `recording`-status sessions. Repro rate ~10% on macOS Apple Silicon.
>
> **Acceptance criteria**:
> - [ ] Two hotkey presses within 200ms produce exactly one session row.
> - [ ] No `recording`-status orphan rows after rapid start/stop sequences.
> - [ ] New unit test in `useDictation.test.tsx` covering the rapid double-trigger case.
> - [ ] `pnpm check` passes.
>
> **Files likely involved**:
> - `apps/desktop/src/hooks/useDictation.ts`
> - `apps/desktop/src-tauri/src/commands/dictation.rs`
> - `apps/desktop/src/test/tauri-mocks.ts`
>
> **Out of scope**:
> - Refactoring the slot config storage.
> - Renaming the `start_dictation` Tauri command.
> - Recording-indicator overlay changes.
>
> **Verification**:
> ```
> pnpm check
> ```
>
> **Risk**: None of the AGENTS.md "Ask first" categories apply. Proceed.
