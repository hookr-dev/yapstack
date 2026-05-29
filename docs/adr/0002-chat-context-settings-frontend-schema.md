# 0002. `chat_context_settings` table via frontend runtime schema (not a numbered migration)

- **Status**: Accepted
- **Date**: 2026-05-28

## Context

The AI Connection/Profile refactor adds a per-chat-context model override: each chat context (a session, folder, "all", "pinned", or "dictation") may pin a specific **Profile**, falling back to the live default **Chat assignment** when unset. This needs a small persisted table:

```sql
CREATE TABLE IF NOT EXISTS chat_context_settings (
  context_key TEXT PRIMARY KEY,
  profile_id  TEXT NULL,        -- NULL / absent ⇒ use the live default
  updated_at  TEXT NOT NULL
);
```

The repo has two schema mechanisms:

1. **Versioned Rust migrations** — `apps/desktop/src-tauri/src/db.rs::migrations()`, run by `tauri-plugin-sql`, tracked in `_sqlx_migrations`. This is the canonical source of truth for fresh installs.
2. **Frontend runtime schema** — `apps/desktop/src/lib/db.ts::ensureRuntimeSchema()`, idempotent `CREATE … IF NOT EXISTS` / `ALTER … ` (`.catch()`-guarded) re-applied on every `getDb()`.

AGENTS.md requires a schema bump to be backed by an ADR + migration plan. AGENTS.md also documents that frontend `getDb()` runtime patches are an accepted mechanism, not a workaround to be avoided.

A prior incident ("ghost v11") left some local dev DBs with an inconsistent `_sqlx_migrations` history: a v11 entry from another branch makes sqlx **silently refuse** any later numbered migration. `segments.speaker_id` was already moved out of the Rust list into `ensureRuntimeSchema()` for exactly this reason — adding it as a numbered migration would never apply on affected DBs.

## Decision

Define `chat_context_settings` in the **frontend** `ensureRuntimeSchema()`, not as a numbered Rust migration.

- Fresh installs get the table on first `getDb()` via `CREATE TABLE IF NOT EXISTS`.
- Affected dev DBs (ghost-v11) get it too, because the frontend path doesn't depend on the sqlx migration ledger.
- The table is additive and standalone (no backfill, no FK), so there is no data-migration step — the "migration plan" is simply "create if absent on next load."

Cascade integrity lives in the app layer: deleting a Connection or Profile clears any `chat_context_settings` row whose `profile_id` references a removed Profile (`clearChatContextProfilesByProfileId`), so chat falls back to the live default rather than pointing at a dead Profile.

## Consequences

- **Works on every DB**, including ghost-v11 dev DBs, with no sqlx ledger risk.
- **Trade-off**: the canonical Rust migration list no longer fully describes the schema. Mitigated by documenting the frontend-created objects in [`API_REFERENCE.md`](../API_REFERENCE.md) (§ Runtime schema patches) and this ADR. This is a pre-existing condition (`segments.speaker_id`, the `audio_save_locations`/recording-sweep patches), not new to this change.
- **Future cleanup**: if the ghost-v11 ledger skew is ever resolved across dev DBs, these frontend-defined objects can be folded back into numbered migrations in one pass. Not worth the risk today.

## References

- AGENTS.md § "Decisions that need a heads-up" (schema migrations) and § "Changelog discipline".
- `apps/desktop/src/lib/db.ts` — `ensureRuntimeSchema()`, `clearChatContextProfilesByProfileId()`.
- `apps/desktop/src-tauri/src/db.rs` — `migrations()` and the `segments.speaker_id` ghost-v11 note.
