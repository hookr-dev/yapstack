import Database from "@tauri-apps/plugin-sql";

// --- Types ---

export type SessionStatus = "recording" | "completed";
export type SessionType = "manual" | "recording";

export interface DbSession {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  source: string;
  status: SessionStatus;
  duration_seconds: number | null;
  total_segments: number;
  folder_id: string | null;
  is_pinned: number;
  pinned_at: string | null;
  session_type: SessionType;
  wav_file_path: string | null;
  wav_duration_seconds: number | null;
  sort_order: number;
}

export interface DbNote {
  id: string;
  session_id: string;
  content: string;
  updated_at: string;
}

export interface DbNoteVersion {
  id: string;
  note_id: string;
  content: string;
  created_at: string;
}

export interface DbSegment {
  id: string;
  session_id: string;
  source: "Mic" | "System";
  text: string;
  audio_offset_seconds: number;
  chunk_duration_seconds: number;
  confidence: number;
  created_at: string;
  chunk_index: number;
  original_text: string | null;
  edited_at: string | null;
  deleted_at: string | null;
  hidden: number;
  // Populated when Parakeet + Sortformer diarization tagged this segment.
  // NULL/undefined for Whisper-transcribed segments and for any row written
  // before migration v11.
  speaker_id?: number | null;
}

export interface DbFolder {
  id: string;
  name: string;
  parent_id: string | null;
  sort_order: number;
  icon: string | null;
  color: string | null;
  description: string | null;
  created_at: string;
  updated_at: string;
}

export interface DbSessionFolder {
  session_id: string;
  folder_id: string;
  created_at: string;
}

export interface DbTag {
  id: string;
  name: string;
  color: string | null;
  created_at: string;
}

export interface DbSessionTag {
  session_id: string;
  tag_id: string;
  source: "manual" | "auto" | "ai";
  confidence: number | null;
  created_at: string;
}

export interface DbChatMessage {
  id: string;
  context_key: string;
  session_id: string | null;
  role: "user" | "assistant";
  content: string;
  action: string | null;
  created_at: string;
}

// --- Singleton ---

let dbInstance: Database | null = null;

async function getDb(): Promise<Database> {
  if (!dbInstance) {
    dbInstance = await Database.load("sqlite:yapstack.db");
    // Idempotent runtime patch: tauri-plugin-sql migrations stop at v10, but
    // segment inserts reference speaker_id. Duplicate-column error is the
    // expected no-op on subsequent runs.
    await dbInstance
      .execute("ALTER TABLE segments ADD COLUMN speaker_id INTEGER")
      .catch(() => {});
  }
  return dbInstance;
}

// --- Session CRUD ---

export async function createSession(
  id: string,
  source: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "INSERT INTO sessions (id, source) VALUES ($1, $2)",
    [id, source],
  );
}

export async function updateSessionTitle(
  id: string,
  title: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "UPDATE sessions SET title = $1, updated_at = datetime('now') WHERE id = $2",
    [title, id],
  );
}

export async function completeSession(
  id: string,
  durationSeconds: number,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    `UPDATE sessions
     SET status = 'completed',
         duration_seconds = $1,
         total_segments = (SELECT COUNT(*) FROM segments WHERE session_id = $2),
         updated_at = datetime('now')
     WHERE id = $2`,
    [durationSeconds, id],
  );
}

export async function deleteSession(id: string): Promise<void> {
  const db = await getDb();
  await db.execute("DELETE FROM sessions WHERE id = $1", [id]);
}

export async function listSessions(): Promise<DbSession[]> {
  const db = await getDb();
  return await db.select<DbSession[]>(
    "SELECT * FROM sessions ORDER BY created_at DESC",
  );
}

export async function getSession(id: string): Promise<DbSession | null> {
  const db = await getDb();
  const rows = await db.select<DbSession[]>(
    "SELECT * FROM sessions WHERE id = $1",
    [id],
  );
  return rows[0] ?? null;
}

export async function deleteAllSessions(): Promise<void> {
  const db = await getDb();
  // CASCADE handles child tables: segments, notes (→ note_versions),
  // chat_messages, session_folders
  await db.execute("DELETE FROM sessions");
}

// --- Pin operations ---

export async function togglePin(id: string): Promise<void> {
  const db = await getDb();
  const rows = await db.select<{ is_pinned: number }[]>(
    "SELECT is_pinned FROM sessions WHERE id = $1",
    [id],
  );
  if (rows.length === 0) return;

  const isPinned = rows[0].is_pinned;
  if (isPinned) {
    await db.execute(
      "UPDATE sessions SET is_pinned = 0, pinned_at = NULL, updated_at = datetime('now') WHERE id = $1",
      [id],
    );
  } else {
    await db.execute(
      "UPDATE sessions SET is_pinned = 1, pinned_at = datetime('now'), updated_at = datetime('now') WHERE id = $1",
      [id],
    );
  }
}

// --- Folder CRUD ---

export async function createFolder(
  id: string,
  name: string,
  parentId: string | null = null,
  icon: string | null = null,
  color: string | null = null,
  description: string | null = null,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "INSERT INTO folders (id, name, parent_id, icon, color, description) VALUES ($1, $2, $3, $4, $5, $6)",
    [id, name, parentId, icon, color, description],
  );
}

export async function updateFolder(
  id: string,
  updates: { name?: string; icon?: string | null; color?: string | null; description?: string | null },
): Promise<void> {
  const db = await getDb();
  const setClauses: string[] = [];
  const params: unknown[] = [];
  let paramIdx = 1;

  if (updates.name !== undefined) {
    setClauses.push(`name = $${paramIdx++}`);
    params.push(updates.name);
  }
  if (updates.icon !== undefined) {
    setClauses.push(`icon = $${paramIdx++}`);
    params.push(updates.icon);
  }
  if (updates.color !== undefined) {
    setClauses.push(`color = $${paramIdx++}`);
    params.push(updates.color);
  }
  if (updates.description !== undefined) {
    setClauses.push(`description = $${paramIdx++}`);
    params.push(updates.description);
  }

  if (setClauses.length === 0) return;

  setClauses.push(`updated_at = datetime('now')`);
  params.push(id);

  await db.execute(
    `UPDATE folders SET ${setClauses.join(", ")} WHERE id = $${paramIdx}`,
    params,
  );
}

export async function deleteFolder(id: string): Promise<void> {
  const db = await getDb();
  // Clean up chat messages for this folder context
  await db.execute(
    "DELETE FROM chat_messages WHERE context_key = $1",
    [`folder:${id}`],
  );
  // Junction rows cascade, sessions.folder_id set NULL via ON DELETE SET NULL
  await db.execute("DELETE FROM folders WHERE id = $1", [id]);
}

export async function updateFolderParent(
  folderId: string,
  newParentId: string | null,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "UPDATE folders SET parent_id = $1, updated_at = datetime('now') WHERE id = $2",
    [newParentId, folderId],
  );
}

export async function listFolders(): Promise<DbFolder[]> {
  const db = await getDb();
  return await db.select<DbFolder[]>(
    "SELECT * FROM folders ORDER BY sort_order ASC, name ASC",
  );
}

// --- Session-Folder junction ---

export async function addSessionToFolder(
  sessionId: string,
  folderId: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "INSERT OR IGNORE INTO session_folders (session_id, folder_id) VALUES ($1, $2)",
    [sessionId, folderId],
  );
}

export async function removeSessionFromFolder(
  sessionId: string,
  folderId: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "DELETE FROM session_folders WHERE session_id = $1 AND folder_id = $2",
    [sessionId, folderId],
  );
}

export async function removeSessionFromAllFolders(
  sessionId: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "DELETE FROM session_folders WHERE session_id = $1",
    [sessionId],
  );
}

export async function listAllSessionFolders(): Promise<DbSessionFolder[]> {
  const db = await getDb();
  return await db.select<DbSessionFolder[]>(
    "SELECT * FROM session_folders",
  );
}

// --- Tag CRUD ---

export async function createTag(
  id: string,
  name: string,
  color: string | null = null,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "INSERT INTO tags (id, name, color) VALUES ($1, $2, $3)",
    [id, name, color],
  );
}

export async function listTags(): Promise<DbTag[]> {
  const db = await getDb();
  return await db.select<DbTag[]>("SELECT * FROM tags ORDER BY name");
}

export async function getTagByName(name: string): Promise<DbTag | null> {
  const db = await getDb();
  const rows = await db.select<DbTag[]>(
    "SELECT * FROM tags WHERE name = $1 COLLATE NOCASE LIMIT 1",
    [name],
  );
  return rows[0] ?? null;
}

export async function deleteTag(id: string): Promise<void> {
  const db = await getDb();
  await db.execute("DELETE FROM tags WHERE id = $1", [id]);
}

export async function addSessionTag(
  sessionId: string,
  tagId: string,
  source: "manual" | "auto" | "ai" = "manual",
  confidence: number | null = null,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "INSERT OR IGNORE INTO session_tags (session_id, tag_id, source, confidence) VALUES ($1, $2, $3, $4)",
    [sessionId, tagId, source, confidence],
  );
}

export async function removeSessionTag(
  sessionId: string,
  tagId: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "DELETE FROM session_tags WHERE session_id = $1 AND tag_id = $2",
    [sessionId, tagId],
  );
}

export async function listAllSessionTags(): Promise<DbSessionTag[]> {
  const db = await getDb();
  return await db.select<DbSessionTag[]>("SELECT * FROM session_tags");
}

export async function getSessionTagIds(sessionId: string): Promise<string[]> {
  const db = await getDb();
  const rows = await db.select<{ tag_id: string }[]>(
    "SELECT tag_id FROM session_tags WHERE session_id = $1",
    [sessionId],
  );
  return rows.map((r) => r.tag_id);
}

export async function getSessionTagRows(
  sessionId: string,
): Promise<DbSessionTag[]> {
  const db = await getDb();
  return await db.select<DbSessionTag[]>(
    "SELECT * FROM session_tags WHERE session_id = $1",
    [sessionId],
  );
}

// --- Segment CRUD ---

export async function insertSegment(segment: DbSegment): Promise<void> {
  const db = await getDb();
  await db.execute(
    `INSERT INTO segments (id, session_id, source, text, audio_offset_seconds, chunk_duration_seconds, confidence, chunk_index, speaker_id)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
    [
      segment.id,
      segment.session_id,
      segment.source,
      segment.text,
      segment.audio_offset_seconds,
      segment.chunk_duration_seconds,
      segment.confidence,
      segment.chunk_index,
      segment.speaker_id ?? null,
    ],
  );
}

export async function getSessionSegments(
  sessionId: string,
): Promise<DbSegment[]> {
  const db = await getDb();
  return await db.select<DbSegment[]>(
    "SELECT * FROM segments WHERE session_id = $1 AND deleted_at IS NULL ORDER BY audio_offset_seconds ASC",
    [sessionId],
  );
}

// --- Segment editing ---

export async function updateSegmentText(
  id: string,
  newText: string,
): Promise<void> {
  const db = await getDb();
  // Preserve original text on first edit
  await db.execute(
    `UPDATE segments
     SET original_text = CASE WHEN original_text IS NULL THEN text ELSE original_text END,
         text = $1,
         edited_at = datetime('now')
     WHERE id = $2`,
    [newText, id],
  );
}

export async function softDeleteSegment(id: string): Promise<void> {
  const db = await getDb();
  await db.execute(
    "UPDATE segments SET deleted_at = datetime('now') WHERE id = $1",
    [id],
  );
}

export async function toggleSegmentHidden(id: string): Promise<void> {
  const db = await getDb();
  await db.execute(
    "UPDATE segments SET hidden = CASE WHEN hidden = 0 THEN 1 ELSE 0 END WHERE id = $1",
    [id],
  );
}

export async function softDeleteSegments(ids: string[]): Promise<void> {
  if (ids.length === 0) return;
  const db = await getDb();
  const placeholders = ids.map((_, i) => `$${i + 1}`).join(",");
  await db.execute(
    `UPDATE segments SET deleted_at = datetime('now') WHERE id IN (${placeholders})`,
    ids,
  );
}

export async function setSegmentsHidden(
  ids: string[],
  hidden: boolean,
): Promise<void> {
  if (ids.length === 0) return;
  const db = await getDb();
  const placeholders = ids.map((_, i) => `$${i + 2}`).join(",");
  await db.execute(
    `UPDATE segments SET hidden = $1 WHERE id IN (${placeholders})`,
    [hidden ? 1 : 0, ...ids],
  );
}

// --- Notes CRUD ---

export async function getNote(sessionId: string): Promise<DbNote | null> {
  const db = await getDb();
  const rows = await db.select<DbNote[]>(
    "SELECT * FROM notes WHERE session_id = $1",
    [sessionId],
  );
  return rows[0] ?? null;
}

export async function saveNote(
  sessionId: string,
  content: string,
): Promise<void> {
  const db = await getDb();
  const existing = await getNote(sessionId);
  if (existing) {
    await db.execute(
      "UPDATE notes SET content = $1, updated_at = datetime('now') WHERE id = $2",
      [content, existing.id],
    );
  } else {
    const id = crypto.randomUUID();
    await db.execute(
      "INSERT INTO notes (id, session_id, content) VALUES ($1, $2, $3)",
      [id, sessionId, content],
    );
  }
}

export async function createNoteVersion(
  noteId: string,
  content: string,
): Promise<void> {
  const db = await getDb();
  const id = crypto.randomUUID();
  await db.execute(
    "INSERT INTO note_versions (id, note_id, content) VALUES ($1, $2, $3)",
    [id, noteId, content],
  );
}

export async function getNoteVersions(
  noteId: string,
): Promise<DbNoteVersion[]> {
  const db = await getDb();
  return await db.select<DbNoteVersion[]>(
    "SELECT * FROM note_versions WHERE note_id = $1 ORDER BY created_at DESC",
    [noteId],
  );
}

// --- Search ---

export interface SearchResult {
  type: "segment" | "note" | "session";
  sessionId: string;
  sessionTitle: string;
  snippet: string;
}

export interface DictationSearchResult {
  dictationId: string;
  slotName: string;
  snippet: string;
  sessionId: string | null;
}

function stripHtml(html: string): string {
  return html.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim();
}

export async function searchSegments(
  query: string,
): Promise<SearchResult[]> {
  const db = await getDb();
  const pattern = `%${query}%`;
  const rows = await db.select<
    { session_id: string; text: string; title: string }[]
  >(
    `SELECT seg.session_id, seg.text, s.title
     FROM segments seg
     JOIN sessions s ON seg.session_id = s.id
     WHERE seg.deleted_at IS NULL AND seg.text LIKE $1
     ORDER BY seg.audio_offset_seconds ASC
     LIMIT 50`,
    [pattern],
  );
  return rows.map((r) => ({
    type: "segment",
    sessionId: r.session_id,
    sessionTitle: r.title || "Untitled",
    snippet: r.text,
  }));
}

export async function searchNotes(
  query: string,
): Promise<SearchResult[]> {
  const db = await getDb();
  const pattern = `%${query}%`;
  const rows = await db.select<
    { session_id: string; content: string; title: string }[]
  >(
    `SELECT n.session_id, n.content, s.title
     FROM notes n
     JOIN sessions s ON n.session_id = s.id
     WHERE n.content LIKE $1
     ORDER BY n.updated_at DESC
     LIMIT 50`,
    [pattern],
  );
  return rows.map((r) => ({
    type: "note",
    sessionId: r.session_id,
    sessionTitle: r.title || "Untitled",
    snippet: stripHtml(r.content),
  }));
}

export async function searchSessionsByTitle(
  query: string,
): Promise<SearchResult[]> {
  const db = await getDb();
  const pattern = `%${query}%`;
  const rows = await db.select<{ id: string; title: string }[]>(
    `SELECT id, title FROM sessions
     WHERE title LIKE $1
     ORDER BY updated_at DESC
     LIMIT 20`,
    [pattern],
  );
  return rows.map((r) => ({
    type: "session",
    sessionId: r.id,
    sessionTitle: r.title || "Untitled",
    snippet: "",
  }));
}

export async function searchFolders(
  query: string,
): Promise<{ id: string; name: string }[]> {
  const db = await getDb();
  const pattern = `%${query}%`;
  return await db.select<{ id: string; name: string }[]>(
    `SELECT id, name FROM folders WHERE name LIKE $1 ORDER BY name ASC LIMIT 20`,
    [pattern],
  );
}

export async function searchDictations(
  query: string,
): Promise<DictationSearchResult[]> {
  const db = await getDb();
  const pattern = `%${query}%`;
  const rows = await db.select<
    {
      id: string;
      slot_name: string;
      input_text: string;
      output_text: string;
      session_id: string | null;
    }[]
  >(
    `SELECT id, slot_name, input_text, output_text, session_id
     FROM dictation_history
     WHERE output_text LIKE $1 OR input_text LIKE $1
     ORDER BY created_at DESC
     LIMIT 50`,
    [pattern],
  );
  const q = query.toLowerCase();
  return rows.map((r) => {
    // Prefer the output_text snippet if the match lives there, else fall
    // back to input_text so the user sees which field actually hit.
    const source = r.output_text.toLowerCase().includes(q)
      ? r.output_text
      : r.input_text;
    return {
      dictationId: r.id,
      slotName: r.slot_name,
      snippet: source,
      sessionId: r.session_id,
    };
  });
}

// --- Sort order ---

export async function reorderFolders(
  updates: { id: string; sort_order: number }[],
): Promise<void> {
  const db = await getDb();
  for (const { id, sort_order } of updates) {
    await db.execute(
      "UPDATE folders SET sort_order = $1, updated_at = datetime('now') WHERE id = $2",
      [sort_order, id],
    );
  }
}

// --- WAV file ---

export async function updateSessionWavPath(
  id: string,
  wavFilePath: string,
  wavDurationSeconds: number,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "UPDATE sessions SET wav_file_path = $1, wav_duration_seconds = $2, updated_at = datetime('now') WHERE id = $3",
    [wavFilePath, wavDurationSeconds, id],
  );
}

// --- Manual session ---

export async function createManualSession(
  id: string,
  title: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "INSERT INTO sessions (id, title, source, status, session_type) VALUES ($1, $2, 'MicOnly', 'completed', 'manual')",
    [id, title],
  );
}

// --- Chat messages ---

export async function insertChatMessage(msg: DbChatMessage): Promise<void> {
  const db = await getDb();
  await db.execute(
    "INSERT INTO chat_messages (id, context_key, session_id, role, content, action) VALUES ($1, $2, $3, $4, $5, $6)",
    [msg.id, msg.context_key, msg.session_id, msg.role, msg.content, msg.action],
  );
}

export async function getChatMessages(
  contextKey: string,
): Promise<DbChatMessage[]> {
  const db = await getDb();
  return await db.select<DbChatMessage[]>(
    "SELECT * FROM chat_messages WHERE context_key = $1 ORDER BY created_at ASC",
    [contextKey],
  );
}

export async function updateChatMessageContent(
  id: string,
  content: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "UPDATE chat_messages SET content = $1 WHERE id = $2",
    [content, id],
  );
}

export async function deleteChatMessages(
  contextKey: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "DELETE FROM chat_messages WHERE context_key = $1",
    [contextKey],
  );
}

// --- Dictation history ---

export interface DbDictationHistory {
  id: string;
  slot_id: string;
  slot_name: string;
  input_text: string;
  output_text: string;
  ai_enabled: number;
  ai_prompt: string | null;
  output_action: string;
  wav_file_path: string | null;
  wav_duration_seconds: number | null;
  session_id: string | null;
  created_at: string;
}

export async function insertDictationHistory(
  entry: Omit<DbDictationHistory, "created_at">,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    `INSERT INTO dictation_history (id, slot_id, slot_name, input_text, output_text, ai_enabled, ai_prompt, output_action, wav_file_path, wav_duration_seconds, session_id)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)`,
    [
      entry.id,
      entry.slot_id,
      entry.slot_name,
      entry.input_text,
      entry.output_text,
      entry.ai_enabled ? 1 : 0,
      entry.ai_prompt,
      entry.output_action,
      entry.wav_file_path,
      entry.wav_duration_seconds,
      entry.session_id,
    ],
  );
}

export async function listDictationHistory(
  limit = 200,
): Promise<DbDictationHistory[]> {
  const db = await getDb();
  return await db.select<DbDictationHistory[]>(
    "SELECT * FROM dictation_history ORDER BY created_at DESC LIMIT $1",
    [limit],
  );
}

export async function getDictationHistoryEntry(
  id: string,
): Promise<DbDictationHistory | null> {
  const db = await getDb();
  const rows = await db.select<DbDictationHistory[]>(
    "SELECT * FROM dictation_history WHERE id = $1",
    [id],
  );
  return rows[0] ?? null;
}

export async function deleteDictationHistoryEntry(
  id: string,
): Promise<void> {
  const db = await getDb();
  await db.execute("DELETE FROM dictation_history WHERE id = $1", [id]);
}

export async function clearDictationHistory(): Promise<void> {
  const db = await getDb();
  await db.execute("DELETE FROM dictation_history");
}

export async function updateDictationHistorySessionId(
  id: string,
  sessionId: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "UPDATE dictation_history SET session_id = $1 WHERE id = $2",
    [sessionId, id],
  );
}

// --- Dictation session link cleanup ---

export async function clearDictationSessionLink(
  sessionId: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "UPDATE dictation_history SET session_id = NULL WHERE session_id = $1",
    [sessionId],
  );
}

// --- Multi-session helpers ---

export interface SessionWithNote {
  sessionId: string;
  title: string;
  createdAt: string;
  noteContent: string | null;
}

export async function getNotesForSessions(
  sessionIds: string[],
): Promise<SessionWithNote[]> {
  if (sessionIds.length === 0) return [];
  const db = await getDb();
  const placeholders = sessionIds.map((_, i) => `$${i + 1}`).join(", ");
  return await db.select<SessionWithNote[]>(
    `SELECT s.id as sessionId, s.title, s.created_at as createdAt, n.content as noteContent
     FROM sessions s
     LEFT JOIN notes n ON n.session_id = s.id
     WHERE s.id IN (${placeholders})
     ORDER BY s.created_at DESC`,
    sessionIds,
  );
}
