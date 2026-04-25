import { describe, it, expect, vi } from "vitest";

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
} from "./ai-tools";
import type { DbSegment } from "./db";

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
    expect(names).not.toContain("add_to_folder");
    expect(names).not.toContain("get_folder_context");
  });
});

