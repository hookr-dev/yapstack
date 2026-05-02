import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock db.ts to prevent Tauri import chain
vi.mock("@/lib/db", () => ({
  updateSessionTitle: vi.fn(),
  getNote: vi.fn(),
  saveNote: vi.fn(),
  createNoteVersion: vi.fn(),
  togglePin: vi.fn(),
  getSession: vi.fn(),
  getTagByName: vi.fn(),
  createTag: vi.fn(),
  addSessionTag: vi.fn(),
  removeSessionTag: vi.fn(),
  addSessionToFolder: vi.fn(),
  removeSessionFromFolder: vi.fn(),
  listFolders: vi.fn().mockResolvedValue([]),
  // Bulk lookups consumed by search_semantic enrichment. Each test
  // overrides these with mockResolvedValueOnce; the fallbacks here keep
  // any test that doesn't care about enrichment from blowing up.
  searchSegments: vi.fn().mockResolvedValue([]),
  searchNotes: vi.fn().mockResolvedValue([]),
  searchSessionsByTitle: vi.fn().mockResolvedValue([]),
  searchDictations: vi.fn().mockResolvedValue([]),
  getSessionsByIds: vi.fn().mockResolvedValue([]),
  updateSegmentText: vi.fn(),
  listAllSessionFolders: vi.fn().mockResolvedValue([]),
  getSessionSegments: vi.fn().mockResolvedValue([]),
  getSegmentsByIds: vi.fn().mockResolvedValue([]),
  getDictationsByIds: vi.fn().mockResolvedValue([]),
  getNotesByIds: vi.fn().mockResolvedValue([]),
  getSessionTagRows: vi.fn().mockResolvedValue([]),
}));

// search_semantic talks to the Rust commands surface; mock the read-side
// shape and override per-test to drive scope-filter scenarios.
vi.mock("@/lib/tauri", () => ({
  commands: {
    embeddingModelStatus: vi.fn().mockResolvedValue({
      status: "ok",
      data: {
        ready: true,
        model_name: "bge-small-en-v1.5",
        model_version: "1.5.0",
        dimensions: 384,
      },
    }),
    searchSemantic: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
  },
}));

// useAppStore.getState().settings → settings used inside search_semantic.
// We force the English + enabled path by default; tests that need the
// non-English / disabled branch override per-test.
vi.mock("@/stores/appStore", () => ({
  useAppStore: {
    getState: () => ({
      settings: { embeddingsEnabled: true, language: "en" },
    }),
  },
}));

vi.mock("@/lib/folder-tree", () => ({
  findBranchConflicts: vi.fn().mockReturnValue([]),
  getFolderPath: vi.fn().mockReturnValue([]),
  buildFolderTree: vi.fn().mockReturnValue([]),
}));

vi.mock("@/lib/ai", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./ai")>();
  return {
    ...actual,
    markdownToBasicHtml: vi.fn((s: string) => `<p>${s}</p>`),
    assembleFolderTreeContext: vi.fn().mockReturnValue(""),
  };
});

import {
  convertCitationsToSegmentRefs,
  getRegisteredTools,
  executeTool,
  type ToolContext,
} from "./ai-tools";
import type { DbSegment } from "./db";
import { commands } from "@/lib/tauri";
import * as db from "@/lib/db";

function makeSegment(overrides: Partial<DbSegment> & { id: string }): DbSegment {
  return {
    session_id: "s1",
    source: "Mic",
    text: "test",
    audio_offset_seconds: 0,
    chunk_duration_seconds: 5,
    confidence: 0.9,
    created_at: "",
    chunk_index: 0,
    original_text: null,
    edited_at: null,
    deleted_at: null,
    hidden: 0,
    ...overrides,
  };
}

describe("convertCitationsToSegmentRefs", () => {
  it("passes through text without citations", () => {
    expect(convertCitationsToSegmentRefs("Hello world", [])).toBe(
      "Hello world",
    );
  });

  it("replaces a single citation with matched segment", () => {
    const segments = [makeSegment({ id: "abc123", audio_offset_seconds: 65 })];
    const result = convertCitationsToSegmentRefs(
      "See [[seg:abc123]]",
      segments,
    );
    expect(result).toContain('data-segment-id="abc123"');
    expect(result).toContain('data-timestamp="1:05"');
    expect(result).toContain('data-offset="65"');
  });

  it("uses truncated ID for unmatched segment", () => {
    const result = convertCitationsToSegmentRefs(
      "See [[seg:unknown123]]",
      [],
    );
    expect(result).toContain('data-segment-id="unknown123"');
    expect(result).toContain(">unknown1</span>");
  });

  it("replaces multiple citations", () => {
    const segments = [
      makeSegment({ id: "a1", audio_offset_seconds: 0 }),
      makeSegment({ id: "b2", audio_offset_seconds: 30 }),
    ];
    const result = convertCitationsToSegmentRefs(
      "First [[seg:a1]] and second [[seg:b2]]",
      segments,
    );
    expect(result).toContain('data-segment-id="a1"');
    expect(result).toContain('data-segment-id="b2"');
  });

  it("handles citation at start and end", () => {
    const segments = [makeSegment({ id: "x1", audio_offset_seconds: 0 })];
    const result = convertCitationsToSegmentRefs("[[seg:x1]]", segments);
    expect(result).toContain("data-segment-ref");
    expect(result).not.toContain("[[seg:");
  });
});

describe("getRegisteredTools", () => {
  it("returns the registered tools", () => {
    const tools = getRegisteredTools();
    const names = tools.map((t) =>
      t.type === "function" ? t.function.name : "",
    );
    expect(names).toContain("update_title");
    expect(names).toContain("save_to_notes");
    expect(names).toContain("pin_session");
    expect(names).toContain("tag_session");
    expect(names).toContain("search_folders");
    expect(names).toContain("add_session_to_folder");
    expect(names).toContain("search_semantic");
    expect(names).not.toContain("add_to_folder");
    expect(names).not.toContain("get_folder_context");
  });
});

describe("search_semantic scope filtering", () => {
  beforeEach(() => {
    // Re-establish empty defaults after each test. mockReset wipes the
    // implementation entirely (returns undefined), which would break
    // .map() in the executor — so we re-seed [] / { ok: [] } instead.
    vi.mocked(commands.searchSemantic).mockReset();
    vi.mocked(commands.searchSemantic).mockResolvedValue({
      status: "ok",
      data: [],
    });
    vi.mocked(commands.embeddingModelStatus).mockResolvedValue({
      status: "ok",
      data: {
        ready: true,
        model_name: "bge-small-en-v1.5",
        model_version: "1.5.0",
        dimensions: 384,
      },
    });
    vi.mocked(db.getSegmentsByIds).mockReset();
    vi.mocked(db.getSegmentsByIds).mockResolvedValue([]);
    vi.mocked(db.getDictationsByIds).mockReset();
    vi.mocked(db.getDictationsByIds).mockResolvedValue([]);
    vi.mocked(db.getNotesByIds).mockReset();
    vi.mocked(db.getNotesByIds).mockResolvedValue([]);
  });

  it("multi-session chat: drops hits whose source session is not in allowedSessionIds", async () => {
    // Three Segment hits across three different sessions; the chat scope
    // only allows session "in-1" and "in-2". The "leak" session must be
    // dropped before the model sees it.
    vi.mocked(commands.searchSemantic).mockResolvedValueOnce({
      status: "ok",
      data: [
        { source_id: "seg-in-1", source_kind: "Segment", distance: 0.1 },
        { source_id: "seg-leak", source_kind: "Segment", distance: 0.15 },
        { source_id: "seg-in-2", source_kind: "Segment", distance: 0.2 },
      ],
    });
    vi.mocked(db.getSegmentsByIds).mockResolvedValueOnce([
      makeSegment({ id: "seg-in-1", session_id: "in-1", text: "alpha" }),
      makeSegment({ id: "seg-leak", session_id: "leak", text: "beta" }),
      makeSegment({ id: "seg-in-2", session_id: "in-2", text: "gamma" }),
    ]);

    const ctx: ToolContext = {
      scope: "retrieval",
      allowedSessionIds: ["in-1", "in-2"],
    };
    const result = await executeTool(
      "search_semantic",
      { query: "anything", limit: 10, surfaces: ["segment"] },
      ctx,
    );
    expect(result).not.toBeNull();
    expect(result!.result).toContain("seg-in-1");
    expect(result!.result).toContain("seg-in-2");
    expect(result!.result).not.toContain("seg-leak");
    expect(result!.observation?.affectedIds).toEqual(["seg-in-1", "seg-in-2"]);
  });

  it("dictation chat: blocks Segment / Note even when args.surfaces requests them", async () => {
    // The dictation context's surfaceFilter is ["Dictation"]. Even if the
    // model passes surfaces=["segment","note"], the request must be
    // narrowed by intersection — and since the result is empty, the tool
    // returns the "Surfaces blocked" message instead of calling the
    // backend at all.
    const ctx: ToolContext = {
      scope: "retrieval",
      allowedSessionIds: [],
      surfaceFilter: ["Dictation"],
    };
    const result = await executeTool(
      "search_semantic",
      { query: "anything", limit: 10, surfaces: ["segment", "note"] },
      ctx,
    );
    expect(result).not.toBeNull();
    expect(result!.detail).toBe("Surfaces blocked");
    expect(commands.searchSemantic).not.toHaveBeenCalled();
  });

  it("dictation chat: allows dictation hits but never enriches segments or notes", async () => {
    // Even if the backend KNN somehow returned mixed surfaces (defense
    // in depth), the dictation context still narrows to Dictation only
    // before passing to the backend.
    vi.mocked(commands.searchSemantic).mockResolvedValueOnce({
      status: "ok",
      data: [
        { source_id: "dict-1", source_kind: "Dictation", distance: 0.1 },
      ],
    });
    vi.mocked(db.getDictationsByIds).mockResolvedValueOnce([
      {
        id: "dict-1",
        slot_id: "s1",
        slot_name: "Notes",
        input_text: "raw",
        output_text: "polished",
        ai_enabled: 0,
        ai_prompt: null,
        output_action: "PASTE",
        wav_file_path: null,
        wav_duration_seconds: null,
        session_id: null,
        created_at: "",
      },
    ]);

    const ctx: ToolContext = {
      scope: "retrieval",
      allowedSessionIds: [],
      surfaceFilter: ["Dictation"],
    };
    const result = await executeTool(
      "search_semantic",
      { query: "anything", limit: 10, surfaces: ["dictation"] },
      ctx,
    );
    // Backend was called with the dictation surface only. The 4th arg
    // is the allowed_session_ids — null for the dictation chat case
    // (empty allow-list collapsed to null since Rust treats empty as
    // "no session clamp" anyway, and null skips lifecycle filtering
    // on segments which we don't need here).
    expect(commands.searchSemantic).toHaveBeenCalledWith(
      "anything",
      expect.any(Number),
      ["Dictation"],
      null,
    );
    expect(result).not.toBeNull();
    // The dictation context's empty allowedSessionIds disables the
    // session allow-list (no session-id clamp), so dictation hits — which
    // commonly have session_id=null — flow through. The surfaceFilter is
    // what fences out segment / note results, verified above.
    expect(result!.result).toContain("dict-1");
    expect(result!.result).toContain("polished");
  });

  it("session chat: only returns hits that link to the session", async () => {
    // Session-scoped chat: allowedSessionIds is implicitly [sessionId]
    // (the search_semantic executor synthesizes it). Hits in other
    // sessions must be dropped.
    vi.mocked(commands.searchSemantic).mockResolvedValueOnce({
      status: "ok",
      data: [
        { source_id: "seg-mine", source_kind: "Segment", distance: 0.1 },
        { source_id: "seg-other", source_kind: "Segment", distance: 0.15 },
      ],
    });
    vi.mocked(db.getSegmentsByIds).mockResolvedValueOnce([
      makeSegment({ id: "seg-mine", session_id: "MY-SESSION", text: "ok" }),
      makeSegment({ id: "seg-other", session_id: "ELSEWHERE", text: "leak" }),
    ]);

    const ctx: ToolContext = {
      scope: "session",
      sessionId: "MY-SESSION",
      currentTitle: "t",
      currentNote: null,
      isPinned: false,
    };
    const result = await executeTool(
      "search_semantic",
      { query: "anything", limit: 10, surfaces: ["segment"] },
      ctx,
    );
    expect(result).not.toBeNull();
    expect(result!.result).toContain("seg-mine");
    expect(result!.result).not.toContain("seg-other");
  });
});

