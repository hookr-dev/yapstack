# Shared Scheduler: Concurrent Dictation + Live Session

## Context

Today, starting dictation while a session is recording fails — the
`TranscriptionClient` is exclusively taken by `live_transcription_loop`
and the dictation hotkey hits "transcription client not initialized".
This is called out as a known gap in `IMPLEMENTATION_LOG.md`'s "What's
Not Yet Built" section.

The `TranscriptionScheduler` (landed on `feat/transcription-scheduler`)
already serializes work through the sidecar with priority ordering. It
was built per-session, but with modest changes it can become the
long-lived hub that both the session live loop and the dictation hook
submit into — with dictation getting its own priority tier so the
user's hotkey audio doesn't wait behind a 10 s session chunk.

## Prerequisites

Do **not** start this work on top of open feature branches. Merge
order:

1. `feat/transcription-scheduler` → `main`
2. `feat/silero-vad-parakeet` → `main` (depends on #1 in spirit; the
   Silero work doesn't touch scheduler ownership but the integration
   test surface overlaps)
3. Branch `feat/shared-scheduler-dictation` off the merged `main`

Stacking this on a chain of three unmerged branches would tangle
resolution and make review harder.

## Approach

Split into two landings.

### Landing 1 — foundations (~2.5 hrs, low risk)

Mechanical changes that make the API shape right and unblock
concurrent dictation *as long as* `init_transcription_client` is only
called at startup / engine-swap time (it is, today).

**Phase A — long-lived scheduler in shared state.**
Current:
```rust
pub type TranscriptionClientState = Arc<Mutex<Option<TranscriptionClient>>>;
```
Target:
```rust
pub type TranscriptionClientState = Arc<Mutex<Option<Arc<TranscriptionScheduler>>>>;
```

- `init_transcription_client` (`transcription.rs`): after spawning the
  client, wrap it in a scheduler and store `Arc<TranscriptionScheduler>`.
  Today this work happens per-session inside `start_live_transcription`
  — move it up.
- `start_live_transcription` (`live_transcription.rs:~2798`): stop
  calling `take()` on the shared state. Clone the `Arc<Scheduler>` and
  pass it to `TranscriptionContext`. Drop the "return client on exit"
  cleanup — nothing to return.
- `TranscriptionContext.scheduler` stays an `Arc<TranscriptionScheduler>`.
  It's already cloneable, so two loops can hold clones.
- `shutdown_transcription_client` (`transcription.rs`): call
  `scheduler.shutdown_and_return()` explicitly (or add a separate
  teardown). Clear the shared state Option.

**Phase C — reroute `transcribe_audio`.**
The one-shot `transcribe_audio` command currently locks the shared
state and calls `client.transcribe()` directly. Update it to submit a
`JobOrigin::Dictation` (from Phase E) job to the scheduler and await
the oneshot. ~30 min.

**Phase E — `JobOrigin::Dictation` priority tier.**

`transcription_scheduler.rs`:
```rust
pub enum JobOrigin {
    Live,
    Dictation,   // NEW
    Backfill,
    FinalFlush,
}
```

Priority ordering in `SchedulerQueues::pick_next`:
`FinalFlush > Live (mic/system round-robin) > Dictation > Backfill`.

Rationale: an active session's live mic/system chunks rank above
dictation — the user consented to recording the session and expects
its transcript to be timely. Dictation is usually short (<5 s) and
users tolerate a ~1–2 s lead-in if they happened to trigger it mid-chunk.
Above Backfill because dictation is user-present; backfill is
background.

Wire: `useDictation.ts` unchanged on the frontend (still calls
`start_live_transcription` with a synthetic session id). Inside the
live loop, when the session id matches the dictation id, stamp
outgoing jobs with `JobOrigin::Dictation` instead of `JobOrigin::Live`.
Or simpler: add a `LiveTranscriptionConfig.priority` field that
distinguishes session vs dictation sources without string-matching.

**Scope after Landing 1:** a single session + one dictation at a time
can coexist. Trying to swap engines during an active session still
errors out (as it does today). Init guard stays.

### Landing 2 — engine-swap support (~3.5 hrs, medium risk)

Only needed if users report wanting to change models mid-session, or
if `switchEngine` starts to fire during a session (it doesn't today —
the frontend disables it).

**Phase B — remove/narrow `init_transcription_client` guard.**
Today the guard errors on double-init. With a shared scheduler, we
need to support:
- Starting fresh on app launch.
- Replacing the client on engine swap while no session is running.
- Rejecting engine swap while a session *is* running, with a clear
  error (or: drain + swap, see Phase D).

**Phase D — scheduler drain + swap workflow.**

New method on the scheduler:
```rust
impl TranscriptionScheduler {
    pub async fn replace_client(&self, new: TranscriptionClient)
        -> Result<(), SchedulerError>;
}
```

Semantics:
1. Stop accepting new jobs (transient "swapping" state).
2. Drain the current worker (finish any in-flight job, cancel queued).
3. Swap the inner `Arc<TranscriptionClient>` to the new one.
4. Resume accepting jobs.

The existing `respawn_client` is designed for sidecar crashes (same
engine, same model). This is a *different* operation — explicit teardown
of valid state. Worth a separate method to keep the failure modes
distinct.

## Non-goals (explicit)

- Two simultaneous sessions. One session + one dictation covers the
  real-world use case; allowing unbounded concurrent sessions would
  surface audio-routing questions that aren't worth solving yet.
- Preempting an in-flight transcribe. The sidecar's decode step is
  not interruptible; once a job has crossed into the sidecar, we wait
  for its result. The priority queue orders *entry*, not execution.
- Second sidecar process for dictation (memory-doubling; separate
  model preload). Deferred; revisit only if Landing 1's latency under
  contention proves unacceptable.

## Critical files

Landing 1:

- `apps/desktop/src-tauri/src/commands/transcription.rs` — state type
  definition, `init_transcription_client`, `shutdown_transcription_client`,
  `transcribe_audio`.
- `apps/desktop/src-tauri/src/commands/live_transcription.rs` — drop
  the `take()` + return-client cleanup; accept cloned `Arc<Scheduler>`
  in `TranscriptionContext`.
- `apps/desktop/src-tauri/src/commands/transcription_scheduler.rs` —
  add `JobOrigin::Dictation`, update `SchedulerQueues::pick_next` and
  `push` and `cancel_all` match arms, add `live_dictation` queue field.
- `apps/desktop/src-tauri/src/lib.rs` — state wiring (if the
  constructed type changes shape).

Landing 2:

- Same scheduler module plus a new `replace_client` method.
- `transcription.rs::init_transcription_client` — guard semantics.

## Reused utilities

- `TranscriptionScheduler::new`, `shutdown_and_return` — the existing
  API already suffices for Landing 1 (no internal changes needed
  beyond the `JobOrigin` addition).
- `JobRequest` payload shape — unchanged.
- Audio capture (`AudioManager`, ring buffers) — already multiplexes
  fine; session and dictation both read independently.

## Verification

**Landing 1:**

- Unit: `SchedulerQueues::pick_next` priority table — add Dictation
  test cases covering Live > Dictation > Backfill and
  FinalFlush > Dictation.
- Unit: `JobOrigin::Dictation` round-trips through `push` + `cancel_all`.
- Integration (manual, `pnpm tauri dev`):
  1. Start a session. Let backfill complete. Speak a few live chunks.
  2. Trigger the dictation hotkey. Confirm dictation transcribes
     within ~2 s (bounded by any in-flight session chunk).
  3. Confirm session keeps recording through the dictation.
  4. Release dictation. Confirm both transcripts land in their own
     sessions with no crossed text.
- Telemetry: log job origin in scheduler worker for first few sessions
  to confirm priority ordering matches expectation under real load.

**Landing 2:**

- Unit: `replace_client` drains in-flight job, swaps, resumes.
- Unit: engine-swap during active session is either rejected or
  drained cleanly (pick one based on UX choice).
- Integration: switch engine via settings while a session is active
  (or with a just-finished dictation still emitting segments).

## Open questions

1. **Dictation identity on the scheduler side.** Should the live loop
   tag jobs `Dictation` based on session id matching the dictation id
   (today's pattern), or is a cleaner `LiveTranscriptionConfig.priority`
   enum worth adding? The latter avoids string matching in a hot path.
   Lean toward the explicit field.
2. **Pre-roll during dictation.** Silero VAD pre-roll for dictation is
   250 ms (Parakeet) / 0 ms (Whisper) today. If a session is active
   with Parakeet, dictation inherits those values. Likely fine; verify
   that dictation's synthetic session id doesn't get the
   engine-specific Parakeet `max_chunk_duration` (10 s) — for
   dictation, shorter chunks are probably better. Might warrant a
   dictation-specific tuning preset.
3. **Auto-pause session while dictating?** Product question: if the
   user hits the dictation hotkey, should session recording pause so
   the dictation mic audio doesn't also land in the session? Today
   they'd double-record. Out of scope for this plan; flag for UX.
4. **Retry behavior on scheduler error during dictation.** The
   scheduler retries once after sidecar respawn. Dictation is
   user-present; a ~3 s respawn + retry might feel worse than just
   failing fast with a toast. Consider a per-job retry policy flag.
