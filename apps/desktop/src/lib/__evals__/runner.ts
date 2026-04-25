/**
 * Test-time DB stub backing the eval cases. Implements the subset of the
 * `@/lib/db` surface that Tools touch, against an in-memory state object.
 *
 * Why an in-memory stub instead of mocking each function per case: cases
 * declare the world (sessions, folders, notes, segments) and the runner
 * builds the closures once. Tools call the stub the same way they call
 * the real DB, including for mutations. This makes round-trip assertions
 * (mutation followed by another tool reading the new state) work
 * naturally.
 */
import type {
  DbSession,
  DbFolder,
  DbSessionFolder,
  DbSegment,
  DbTag,
  DbNote,
  DbSessionTag,
  DbDictationHistory,
  SearchResult,
  DictationSearchResult,
} from "@/lib/db";
import type { EvalFixture } from "./types";

export interface EvalDbStub {
  state: {
    sessions: DbSession[];
    folders: DbFolder[];
    sessionFolders: DbSessionFolder[];
    notes: Map<string, string>;
    segments: Map<string, DbSegment[]>;
    tags: DbTag[];
    sessionTags: DbSessionTag[];
    dictations: DbDictationHistory[];
  };
  module: Record<string, unknown>;
}

export function buildEvalDbStub(fixture: EvalFixture): EvalDbStub {
  const state: EvalDbStub["state"] = {
    sessions: fixture.sessions.map((s) => ({ ...s })),
    folders: fixture.folders.map((f) => ({ ...f })),
    sessionFolders: fixture.sessionFolders.map((sf) => ({ ...sf })),
    notes: new Map(Object.entries(fixture.notes)),
    segments: new Map(
      Object.entries(fixture.segments).map(([k, v]) => [k, v.map((s) => ({ ...s }))]),
    ),
    tags: fixture.tags.map((t) => ({ ...t })),
    sessionTags: fixture.sessionTags.map((st) => ({ ...st })),
    dictations: (fixture.dictations ?? []).map((d) => ({ ...d })),
  };

  // Lightweight LIKE substring match used for searches in the stub. The
  // production code uses FTS5; for the eval harness, substring is enough
  // and keeps fixtures readable.
  const matches = (haystack: string | null | undefined, needle: string) =>
    !!haystack && haystack.toLowerCase().includes(needle.toLowerCase().trim());

  const moduleStub: Record<string, unknown> = {
    listSessions: async () => state.sessions.map((s) => ({ ...s })),
    getSession: async (id: string) =>
      state.sessions.find((s) => s.id === id) ?? null,
    getSessionsByIds: async (ids: string[]) =>
      state.sessions.filter((s) => ids.includes(s.id)).map((s) => ({ ...s })),
    updateSessionTitle: async (id: string, title: string) => {
      const s = state.sessions.find((x) => x.id === id);
      if (s) s.title = title;
    },
    togglePin: async (id: string) => {
      const s = state.sessions.find((x) => x.id === id);
      if (s) s.is_pinned = s.is_pinned === 1 ? 0 : 1;
    },

    listFolders: async () => state.folders.map((f) => ({ ...f })),
    listAllSessionFolders: async () =>
      state.sessionFolders.map((sf) => ({ ...sf })),
    addSessionToFolder: async (sessionId: string, folderId: string) => {
      if (
        !state.sessionFolders.some(
          (sf) => sf.session_id === sessionId && sf.folder_id === folderId,
        )
      ) {
        state.sessionFolders.push({
          session_id: sessionId,
          folder_id: folderId,
          created_at: new Date().toISOString(),
        });
      }
    },
    removeSessionFromFolder: async (sessionId: string, folderId: string) => {
      state.sessionFolders = state.sessionFolders.filter(
        (sf) => !(sf.session_id === sessionId && sf.folder_id === folderId),
      );
    },

    getNote: async (sessionId: string): Promise<DbNote | null> => {
      const content = state.notes.get(sessionId);
      return content === undefined
        ? null
        : {
            id: `note-${sessionId}`,
            session_id: sessionId,
            content,
            updated_at: "",
          };
    },
    saveNote: async (sessionId: string, content: string) => {
      state.notes.set(sessionId, content);
    },

    getSessionSegments: async (sessionId: string) =>
      (state.segments.get(sessionId) ?? []).map((s) => ({ ...s })),

    listTags: async () => state.tags.map((t) => ({ ...t })),
    getTagByName: async (name: string) =>
      state.tags.find((t) => t.name.toLowerCase() === name.toLowerCase()) ??
      null,
    createTag: async (id: string, name: string) => {
      state.tags.push({
        id,
        name,
        color: null,
        created_at: new Date().toISOString(),
      });
    },
    addSessionTag: async (
      sessionId: string,
      tagId: string,
      source: "manual" | "auto" | "ai",
    ) => {
      if (
        !state.sessionTags.some(
          (st) => st.session_id === sessionId && st.tag_id === tagId,
        )
      ) {
        state.sessionTags.push({
          session_id: sessionId,
          tag_id: tagId,
          source,
          confidence: null,
          created_at: new Date().toISOString(),
        });
      }
    },
    removeSessionTag: async (sessionId: string, tagId: string) => {
      state.sessionTags = state.sessionTags.filter(
        (st) => !(st.session_id === sessionId && st.tag_id === tagId),
      );
    },
    getSessionTagIds: async (sessionId: string) =>
      state.sessionTags
        .filter((st) => st.session_id === sessionId)
        .map((st) => st.tag_id),
    getSessionTagRows: async (sessionId: string) =>
      state.sessionTags
        .filter((st) => st.session_id === sessionId)
        .map((st) => ({ ...st })),

    // Search functions used by retrieval tools. Substring match in lieu of FTS5.
    searchSegments: async (query: string): Promise<SearchResult[]> => {
      const out: SearchResult[] = [];
      for (const [sid, segs] of state.segments) {
        const session = state.sessions.find((s) => s.id === sid);
        for (const seg of segs) {
          if (seg.deleted_at || seg.hidden) continue;
          if (matches(seg.text, query)) {
            out.push({
              type: "segment",
              sessionId: sid,
              sessionTitle: session?.title ?? "Untitled",
              snippet: seg.text,
              source: seg.source as "Mic" | "System",
              speakerId: seg.speaker_id ?? null,
            });
          }
        }
      }
      return out;
    },
    searchNotes: async (query: string): Promise<SearchResult[]> => {
      const out: SearchResult[] = [];
      for (const [sid, content] of state.notes) {
        const session = state.sessions.find((s) => s.id === sid);
        if (matches(content, query)) {
          out.push({
            type: "note",
            sessionId: sid,
            sessionTitle: session?.title ?? "Untitled",
            snippet: content,
          });
        }
      }
      return out;
    },
    searchSessionsByTitle: async (query: string): Promise<SearchResult[]> =>
      state.sessions
        .filter((s) => matches(s.title, query))
        .map((s) => ({
          type: "session",
          sessionId: s.id,
          sessionTitle: s.title || "Untitled",
          snippet: "",
        })),
    searchFolders: async (query: string) =>
      state.folders
        .filter((f) => matches(f.name, query))
        .map((f) => ({ id: f.id, name: f.name })),
    searchDictations: async (
      query: string,
    ): Promise<DictationSearchResult[]> => {
      const out: DictationSearchResult[] = [];
      for (const d of state.dictations) {
        if (matches(d.output_text, query) || matches(d.input_text, query)) {
          out.push({
            dictationId: d.id,
            slotName: d.slot_name,
            // Mirror the production `searchDictations` which prefers
            // output_text when the match is there, else input_text.
            snippet: matches(d.output_text, query)
              ? d.output_text
              : d.input_text,
            sessionId: d.session_id,
          });
        }
      }
      return out;
    },

    listDictationHistory: async (): Promise<DbDictationHistory[]> =>
      state.dictations.map((d) => ({ ...d })),

    updateSegmentText: async (id: string, newText: string) => {
      for (const segs of state.segments.values()) {
        const seg = segs.find((s) => s.id === id);
        if (seg) {
          if (seg.original_text == null) seg.original_text = seg.text;
          seg.text = newText;
          seg.edited_at = new Date().toISOString();
          return;
        }
      }
    },
  };

  return { state, module: moduleStub };
}
