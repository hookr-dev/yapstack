# [Task]: Define a response policy for chronic live-transcription drain backlog

## Problem (1-3 sentences, no solution)

When the active sidecar runs at sustained RTFx < 1.0 (e.g. Parakeet on a long monologue, or a thermally-throttled Apple Silicon device), the live tier force-drains preserved audio indefinitely. `live_drain_backlog_seconds` grows without bound, the user sees the number climb in the status popover, and there is no automated response — the session continues to accumulate inference debt until the user stops it. Today's behaviour is "preserve audio, surface backlog, end of policy," which is correct as a v1 but leaves the loop open.

## Acceptance criteria (testable checklist)

- [ ] One response policy is selected from the options below and documented in `docs/ARCHITECTURE.md`.
- [ ] The policy is implemented behind a config flag (default off until validated).
- [ ] A synthetic slow-sidecar test runs at RTFx 0.5 for ≥ 60 s of speech and asserts the chosen behaviour fires (e.g. soft-cap warning event emitted, or engine swap completed, or notify-only event emitted).
- [ ] `live-transcription-warning` (or new dedicated event) carries enough detail for the UI to surface what the system did.
- [ ] `pnpm check` passes.

## Files likely involved

- `apps/desktop/src-tauri/src/commands/live_transcription.rs` (force-drain path, `record_live_drain_backlog`, fatal-warning emission)
- `apps/desktop/src-tauri/src/commands/transcription_scheduler.rs` (if engine-swap path is chosen)
- `apps/desktop/src/components/StatusPopover.tsx` (UI surface for the new event)
- `docs/ARCHITECTURE.md` (Pressure metrics section)

## Out of scope (explicit non-goals)

- Changing the priority queue, live-busy gate, or backfill quantum.
- Multi-engine concurrent operation.
- Replacing `lag_seconds` or `live_drain_backlog_seconds` plumbing.
- Cosmetic UI changes outside the new event surface.

## Verification command

`pnpm check`

## Risk level

Low — none of the AGENTS.md Ask-first categories apply. Sidecar IPC contract is unchanged; only a new event variant or warning is added.

## Hypotheses / suspects (optional)

Three candidate response policies, in increasing engineering cost:

1. **Notify-only.** Emit a `live-transcription-warning` once `live_drain_backlog_seconds` crosses a threshold (e.g. 30 s). UI nag, no functional change. Cheapest; defers the real decision.
2. **Soft cap.** When backlog exceeds N seconds, drop the oldest force-drain slice and emit an explicit "audio dropped to recover" event. Re-introduces the old cap-and-drop behaviour, but only as a last resort.
3. **Engine swap.** Fall back to a faster (lower-quality) engine for the duration of the drain, then return to the configured engine once caught up. Highest engineering cost; biggest product question (which engine, when to swap back, how to surface the quality change).

## Related tickets / context (optional)

Relates to the `feat/backfill-scheduler` branch (force-drain mechanism, `live_drain_backlog_*` plumbing). The drain backlog metric was added specifically so this follow-up could be tracked instead of hidden.
