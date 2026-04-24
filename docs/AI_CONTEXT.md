# AI Context & Tooling

How the AI chat learns what's on screen, what tools it can call, and how the system prompt is assembled. Also covers folder-based organizational context and the pending tags schema on `feature/knowledge-management`.

For the streaming + tool-call plumbing and undo machinery, see [`ARCHITECTURE.md`](./ARCHITECTURE.md) § AI Chat & Tool Calling.

---

## Context Surfaces

YapStack has four chat contexts, each built by a dedicated factory in [`apps/desktop/src/lib/ai-context.ts`](../apps/desktop/src/lib/ai-context.ts). The factory returns an `AIContextValue` with sources (what goes into the prompt), tools (what the model can call), and a system-prompt builder.

| Context | Factory (sources) | System prompt | Tools |
| --- | --- | --- | --- |
| Session | `createSessionSources(sessionId, segmentCount, sessionType)` | `createSessionSystemPromptBuilder(sessionId)` → `getSystemPromptWithToolContext` | `update_title`, `save_to_notes`, `pin_session` |
| Multi-session (All / Pinned) | `createMultiSessionSources(sessionIds, count)` | `createMultiSessionSystemPromptBuilder()` → `getMultiSessionSystemPrompt` | none |
| Folder | `createMultiSessionSources(...)` + folder-description layers | `createMultiSessionSystemPromptBuilder(layers)` | none |
| Dictation | `createDictationSources(count)` | `createDictationSystemPromptBuilder()` → `getDictationSystemPrompt` | none |

`resolveListContext(chatContext, deps)` picks the right factory for non-session views. For sessions, the `AIContextProvider` composes the session factories directly.

### `AIContextValue` shape

```ts
interface AIContextValue {
  contextKey: string;                            // stable key for message history
  sources: ContextSource[];                      // togglable prompt-slot assemblers
  toggleSource: (sourceId: string) => void;
  tools: AIContextTools;                         // { availableToolIds, getToolContext, contextType }
  actions: ActionDefinition[];                   // preset directives (`ai-actions.ts`)
  segments: DbSegment[];                         // used for citation → pill conversion
  buildSystemPrompt: SystemPromptBuilder;        // (directive, parts, attachments) => string
  isSessionContext: boolean;
  sessionId: string | null;
  onToolsExecuted: (toolNames: string[]) => Promise<void>;
  placeholder: string;
}
```

A `ContextSource` owns an assembler function. Sources are togglable (`toggleable: true`) in the UI; disabled sources pass `""` to the builder.

---

## System Prompt Assembly

All prompt text lives in [`apps/desktop/src/lib/ai-prompts.ts`](../apps/desktop/src/lib/ai-prompts.ts). The `AIContextValue.buildSystemPrompt` builder calls these.

### Session prompt — `getSystemPromptWithToolContext`

Assembled in this order (see `getSystemPrompt` + the tool-context wrapper):

1. **Directive** — either the `GENERAL_DIRECTIVE` or a user-selected `ActionDefinition` from `ai-actions.ts`.
2. **Citation instruction** (`CITATION_INSTRUCTION`) — added only when transcript text is present. Tells the model to use `[[seg:ID]]` for references.
3. **Speaker instruction** (`SPEAKER_INSTRUCTION`) — added only when diarization data is present (`transcriptHasSpeakers(segments)`). Tells the model to attribute statements to `(Speaker N)` / `(Alice)` labels.
4. **Notes guidance** (`NOTES_GUIDANCE`) — how to choose `append` vs `replace` for `save_to_notes`.
5. **Content-scale guidance** — `getContentScaleGuidance(transcript)` computes a target word count based on transcript length (ratio 0.4 / 0.3 / 0.2 for short / medium / long).
6. **Transcript** — `## Session Transcript\n<assembled>` (segment lines, optionally with `(Speaker)` prefixes).
7. **Notes** — `## Notes\n<text>` (Tiptap HTML → text).
8. **Attached files** — `## Attached Files\n### name\n<content>` per attachment.
9. **Session metadata tail** — current title, pinned state, has-notes boolean.

### Multi-session prompt — `getMultiSessionSystemPrompt`

Tone: "viewing multiple sessions — compare, synthesize, don't use `#` headings." Includes:

1. A short directive.
2. **Organizational context** block when `folderContext` is non-empty — each layer is `- **{name}:** {description}`.
3. **Sessions** context — either titles-only or titles+notes depending on whether the `session-notes` source is enabled.
4. Attached files.

### Dictation prompt — `getDictationSystemPrompt`

Tone: "viewing dictation history." Plain list of entries, no tools, no citations.

---

## Keeping Context Size Down

The prompt can grow fast when sessions are long or folders aggregate many items. Rules we follow today:

- **Transcripts carry IDs, not timestamps.** `assembleTranscriptContext` emits `[seg:ID]` tokens; the model maps these back to moments without consuming tokens on `MM:SS` strings.
- **Speaker prefix only when diarized.** No `(Speaker 1)` on Whisper sessions — that's wasted budget when every line is the same unknown speaker.
- **Notes are passed as text, not HTML.** Tiptap's HTML is stripped before assembly (`assembleNoteContext`).
- **Multi-session aggregation is titles + notes, not transcripts.** If you're viewing a folder with 200 sessions, the prompt lists titles and (optionally) notes — it does **not** concatenate transcripts. Attempting to do so would blow past any reasonable context budget.
- **Folder-description layer is scoped to the active path.** `resolveListContext` walks `getFolderPath(folders, folderId)` and attaches only folders whose `description` is set. Deep empty trees contribute nothing.
- **Content-scale guidance tells the model to self-limit.** For a 5,000-word transcript, the prompt asks for ≤ 1,000 words out — don't also ask for "detailed" or "exhaustive" output in the directive.

**Rule of thumb when adding a field:** if the field pushes a 60-minute session's prompt past the conservative context budget (~100k tokens on a Whisper/Parakeet transcript), gate it behind a user toggle or summarize it server-side first. Don't silently include it.

---

## Tool Registry — The Contract

Registry: [`apps/desktop/src/lib/ai-tools.ts`](../apps/desktop/src/lib/ai-tools.ts). A singleton `Map<name, ToolDefinition>` populated at module-load time by `registerTool(...)` calls.

### `ToolDefinition` shape

```ts
interface ToolDefinition {
  schema: ChatCompletionTool;                                        // OpenAI function-call schema
  execute: (args, ctx: ToolContext) => Promise<ExecutedTool | null>; // returns null to skip (no-op)
  undo:    (undoData, ctx: { sessionId: string }) => Promise<void>;
  affects?: ToolEffect[];                                            // "session-meta" | "notes"
}

interface ToolContext {
  sessionId: string;
  currentTitle: string;
  currentNote: DbNote | null;
  isPinned: boolean;
  segments?: DbSegment[];
}

interface ExecutedTool {
  name: string;
  label: string;       // short badge label ("Title", "Notes", "Pinned")
  detail: string;      // one-line description shown in the toast
  undoData?: unknown;  // opaque blob passed back to undo()
}
```

### Currently registered tools

| Name | Affects | Params | Behavior | Undo |
| --- | --- | --- | --- | --- |
| `update_title` | `session-meta` | `title: string` (clamped to 80 chars) | Calls `updateSessionTitle`. No-op if unchanged. | Restores previous title. |
| `save_to_notes` | `notes` | `content: string` (markdown), `mode: "append" \| "replace"` | Converts markdown → HTML, runs `convertCitationsToSegmentRefs`, then either overwrites or joins with `<hr>` to existing content. | Restores previous note content (or `<p></p>` if there was none). |
| `pin_session` | `session-meta` | `pinned: boolean` | Toggles pin if different from current state. | Restores previous pin state via `togglePin`. |

`getToolsForContext(isSessionContext)` returns the full schema list for session contexts and `[]` for multi-session/dictation contexts. Multi-session context intentionally exposes no tools — a pin / title change at the folder scope is ambiguous (which session?).

### How to add a new tool

1. **Define the schema.** OpenAI function-calling shape. Be strict about `required`.
2. **Implement `execute`.** Read `ctx` for current state, perform the mutation via `lib/db.ts`, return an `ExecutedTool` with `undoData` so it can be reversed. Return `null` for no-ops (don't throw).
3. **Implement `undo`.** Must be idempotent — it runs only if `undoData !== undefined` and the user hits the undo toast within the window.
4. **Register at module bottom:** `registerTool({ schema, execute, undo, affects });`
5. **Citation handling.** If your tool writes markdown that will be saved as HTML (notes, docs), run the output through `convertCitationsToSegmentRefs(html, segments)` so `[[seg:ID]]` becomes a clickable pill.
6. **Expose it.** Add the tool's name to the relevant context's `availableToolIds` in `ai-context.ts` (session contexts currently get all three — extend the array, don't create a new registry).
7. **Declare effects.** Set `affects` so the UI can invalidate the right data after a run: `"session-meta"` invalidates the header, `"notes"` refreshes the note editor.

Undo window is 10 s; the user sees a toast. Your `undo` must work even if the user has navigated away — treat it as a DB mutation, not a UI revert.

### Streaming + tool execution

`streamChatWithTools()` (in `lib/ai.ts`) yields typed `StreamEvent`s:

- `token` — streaming response chunk
- `tool_calls` — the model requested one or more tool calls
- `done` — final assistant message

Consumed in [`apps/desktop/src/hooks/useChatMessages.ts`](../apps/desktop/src/hooks/useChatMessages.ts). On a `tool_calls` event the hook:

1. Fetches fresh `ToolContext` via the context's `getToolContext()`.
2. Runs `executeTool(name, args, ctx)` for each.
3. Shows a `sonner` toast with an "Undo" action wired to `undoToolCalls(executed, ctx)`.
4. Calls `onToolsExecuted(names)` so the context can invalidate derived state.

---

## Folder Hierarchy

Folders are YapStack's only organizational grouping today (see § Tags for pending work).

### Schema (on `main`)

```sql
folders(
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  parent_id    TEXT,                -- null for root; FK to folders(id)
  sort_order   INTEGER,
  icon         TEXT,
  color        TEXT,
  description  TEXT,                -- injected into multi-session system prompt
  created_at   TEXT,
  updated_at   TEXT
);

session_folders(
  session_id   TEXT NOT NULL,       -- FK sessions(id)
  folder_id    TEXT NOT NULL,       -- FK folders(id)
  created_at   TEXT,
  PRIMARY KEY (session_id, folder_id)
);
```

A session can belong to multiple folders (many-to-many). Folder nesting is unbounded in the schema.

### Traversal helpers

[`apps/desktop/src/lib/folder-tree.ts`](../apps/desktop/src/lib/folder-tree.ts) has everything you need — do not reimplement:

| Function | Use |
| --- | --- |
| `buildFolderTree(folders)` | Flat → nested tree for sidebar rendering. |
| `getFolderPath(folders, id)` | Ancestor chain `[root, …, parent, current]` — used for breadcrumbs and for the multi-session prompt's organizational layer. |
| `getRootFolder(folders, id)` | Root of a branch — used for filtering "All" / "Pinned" views. |
| `getAncestorIds(folders, id)` | Ancestors (exclusive). |
| `getDescendantIds(folders, id)` | All descendants (BFS). |
| `buildChildMap(folders)` | Parent → children lookup. |
| `isDescendantOf(folders, a, b)` | Transitive descendant check. |
| `findBranchConflicts(folders, sessionFolderIds, targetId)` | Detects when a session would end up in both an ancestor and descendant — used by drag-drop to warn the user. |
| `getDisplayFolders(...)` | Picks which folder badge to show on a session row depending on the current view context. |

### Folder context for AI

When the chat context is `{ type: "folder", folderId }`, `resolveListContext` walks `getFolderPath` and keeps only folders with a non-empty `description`:

```ts
const layers = folderPath
  .filter((f) => !!f.description)
  .map((f) => ({ name: f.name, description: f.description }));
```

These become the **Organizational context** bullet list in the multi-session system prompt. Prompt-budget note: we pass `name` + `description` only — not icons, colors, sort order, or member counts. Deep trees should surface in prompts as path breadcrumbs, not flattened lists.

---

## Tags (pending — `feature/knowledge-management`)

**Not on `main`.** The `feature/knowledge-management` branch adds a tags primitive, vocabulary hints, multi-turn tool chaining, and auto-folder suggestions. The branch is currently behind main; expect conflicts when bringing it forward. Apply the "merge-before-cleanup" posture from [`PRINCIPLES.md`](./PRINCIPLES.md).

### Proposed schema (migration v11)

Source: `origin/feature/knowledge-management:apps/desktop/src-tauri/src/db.rs` lines 213–236.

```sql
CREATE TABLE tags (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
  color       TEXT,
  created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_tags_name ON tags(name);

CREATE TABLE session_tags (
  session_id  TEXT NOT NULL,
  tag_id      TEXT NOT NULL,
  source      TEXT NOT NULL DEFAULT 'manual',   -- 'manual' | 'auto'
  confidence  REAL,                             -- populated for auto tags
  created_at  TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (session_id, tag_id),
  FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
  FOREIGN KEY (tag_id)     REFERENCES tags(id)     ON DELETE CASCADE
);
CREATE INDEX idx_session_tags_tag     ON session_tags(tag_id);
CREATE INDEX idx_session_tags_session ON session_tags(session_id);
```

Design notes worth preserving on merge:

- Unique tag names are case-insensitive (`COLLATE NOCASE`).
- `session_tags.source` distinguishes user-applied tags from auto-assigned ones so the UI can show confidence or let the user confirm suggestions.
- `ON DELETE CASCADE` on both FKs: deleting a session removes its tag links; deleting a tag removes it from all sessions.

### Auto-tagging (folder-name matching)

`origin/feature/knowledge-management:apps/desktop/src/lib/auto-tag.ts` builds a keyword map from folder names (length ≥ 4) and scans transcript text for whole-word matches. `FolderSuggestionTracker` requires ≥ 2 matches before surfacing a suggestion — this prevents noisy single-mention false positives.

The interesting reuse: folder names double as the vocabulary for auto-categorization. If we ever introduce tags as a separate axis, we should decide whether auto-tag suggests *folders* (current branch) or *tags* (new). Picking folders keeps the organizational hierarchy coherent; picking tags creates a second vocabulary to maintain.

Once the branch lands, this section becomes authoritative and the "pending" framing drops. Until then it is a planning reference — do not rely on these tables existing on `main`.
