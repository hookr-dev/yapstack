# 0003. AI provider keys stay in renderer-persisted settings (for now)

- **Status**: Accepted (interim)
- **Date**: 2026-05-28

## Context

`Connection.apiKey` is stored inside `AIConfig`, which lives in the Zustand
settings store and is persisted wholesale via the `persist` middleware
(`apps/desktop/src/stores/appStore.ts`). The renderer reads the key directly to
construct the OpenAI client, which runs in the webview with
`dangerouslyAllowBrowser: true` (`apps/desktop/src/lib/ai.ts`).

This means provider API keys sit in renderer-accessible persisted state rather
than the OS secret store (macOS Keychain / Windows Credential Manager / libsecret).
The Tauri best-practice posture is to keep secrets in Rust and hand the renderer
only a reference, with the actual provider calls (or at least the key handling)
on the native side.

This is **not new debt introduced by the Connection/Profile refactor.** The
pre-refactor design already persisted `settings.ai.providers[].apiKey` in the
same store; this change carries that pattern forward into the new
`Connection`/`Profile` shape.

## Decision

Keep keys in renderer-persisted settings for this release. Do **not** block the
Connection/Profile + Live Insights work on a keyring migration.

Rationale for deferring:

- **Threat model.** YapStack is a local, single-user desktop app. The keys are
  the user's own provider keys, stored on their own machine. The realistic
  exposure is local disk / a compromised renderer — and a compromised renderer
  could call the same provider commands regardless of where the key is stored,
  so moving the key to Rust is defense-in-depth, not a hard boundary, unless the
  provider call *itself* also moves behind Rust.
- **Scope.** Doing this properly means: a Rust keyring command surface, settings
  storing only a key *reference*, a migration that moves existing plaintext keys
  out of `localStorage` into the keychain (and scrubs the old copy), and
  reworking client construction. That is its own PR with its own migration and
  test plan — folding it into this branch would balloon scope and risk.
- **Privacy posture alignment.** The longer-term sync direction (see project
  notes: on-device merge, blind encrypted relay) already makes client-side key
  management a first-class problem to solve deliberately, not piecemeal here.

## Consequences

- **Accepted, documented debt.** Keys remain in renderer-persisted settings and
  the OpenAI client keeps `dangerouslyAllowBrowser: true`. This ADR is the
  explicit record so it isn't mistaken for an oversight.
- **Follow-up tracked separately.** A future ADR + PR should: move secrets to an
  OS keyring via Rust, store only a reference in settings, migrate + scrub
  existing keys, and move provider client construction (or key injection) to the
  native side. When that lands, supersede this ADR.
- **In the meantime**, deletion flows already strip a Connection's key from
  state on remove, and the earlier `_legacyAi` duplicate-key snapshot was
  removed (persist v31) so keys aren't copied into extra blobs.

## References

- `apps/desktop/src/lib/ai.ts` — `createAIClientForConnection` (`dangerouslyAllowBrowser`).
- `apps/desktop/src/stores/appStore.ts` — `persist` `partialize` (whole-settings persistence).
- Tauri security guidance on keeping secrets out of the webview.
