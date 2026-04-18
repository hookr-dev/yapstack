import { describe, it, expect, vi, afterEach } from "vitest";

// Mock db.ts to prevent Tauri import chain
vi.mock("@/lib/db", () => ({}));
// Mock openai to prevent network imports
vi.mock("openai", () => ({
  default: vi.fn(),
}));

import {
  chatContextKey,
  getModelsForProvider,
  assembleTranscriptContext,
  assembleNoteContext,
  markdownToBasicHtml,
  fetchCustomModels,
  type ChatContext,
} from "./ai";

describe("chatContextKey", () => {
  it("returns sessionId for session context", () => {
    const ctx: ChatContext = { type: "session", sessionId: "abc-123" };
    expect(chatContextKey(ctx)).toBe("abc-123");
  });

  it("returns folder: prefix for folder context", () => {
    const ctx: ChatContext = { type: "folder", folderId: "folder-1" };
    expect(chatContextKey(ctx)).toBe("folder:folder-1");
  });

  it('returns "global" for all context', () => {
    const ctx: ChatContext = { type: "all" };
    expect(chatContextKey(ctx)).toBe("global");
  });

  it('returns "pinned" for pinned context', () => {
    const ctx: ChatContext = { type: "pinned" };
    expect(chatContextKey(ctx)).toBe("pinned");
  });
});

describe("getModelsForProvider", () => {
  it("returns models for openai", () => {
    const models = getModelsForProvider("openai");
    expect(models).not.toBeNull();
    expect(models!.length).toBeGreaterThan(0);
    expect(models!.some((m) => m.id === "gpt-4o")).toBe(true);
  });

  it("returns models for openrouter", () => {
    const models = getModelsForProvider("openrouter");
    expect(models).not.toBeNull();
    expect(models!.some((m) => m.id.includes("claude"))).toBe(true);
  });

  it("returns null for custom provider", () => {
    expect(getModelsForProvider("custom")).toBeNull();
  });
});

describe("assembleTranscriptContext", () => {
  it("returns empty string for no segments", () => {
    expect(assembleTranscriptContext([])).toBe("");
  });

  it("formats a single segment", () => {
    const segments = [
      {
        id: "seg1",
        session_id: "s1",
        source: "Mic" as const,
        text: "Hello world",
        audio_offset_seconds: 65,
        chunk_duration_seconds: 5,
        confidence: 0.9,
        created_at: "",
        chunk_index: 0,
        original_text: null,
        edited_at: null,
        deleted_at: null,
        hidden: 0,
      },
    ];
    expect(assembleTranscriptContext(segments)).toBe("[seg:seg1 1:05] Hello world");
  });

  it("filters hidden segments", () => {
    const segments = [
      {
        id: "seg1",
        session_id: "s1",
        source: "Mic" as const,
        text: "Visible",
        audio_offset_seconds: 0,
        chunk_duration_seconds: 5,
        confidence: 0.9,
        created_at: "",
        chunk_index: 0,
        original_text: null,
        edited_at: null,
        deleted_at: null,
        hidden: 0,
      },
      {
        id: "seg2",
        session_id: "s1",
        source: "Mic" as const,
        text: "Hidden",
        audio_offset_seconds: 5,
        chunk_duration_seconds: 5,
        confidence: 0.9,
        created_at: "",
        chunk_index: 1,
        original_text: null,
        edited_at: null,
        deleted_at: null,
        hidden: 1,
      },
    ];
    expect(assembleTranscriptContext(segments)).toBe("[seg:seg1 0:00] Visible");
  });

  it("filters deleted segments", () => {
    const segments = [
      {
        id: "seg1",
        session_id: "s1",
        source: "Mic" as const,
        text: "Deleted",
        audio_offset_seconds: 0,
        chunk_duration_seconds: 5,
        confidence: 0.9,
        created_at: "",
        chunk_index: 0,
        original_text: null,
        edited_at: null,
        deleted_at: "2024-01-01",
        hidden: 0,
      },
    ];
    expect(assembleTranscriptContext(segments)).toBe("");
  });
});

describe("assembleNoteContext", () => {
  it("strips HTML tags", () => {
    expect(assembleNoteContext("<p>Hello <b>world</b></p>")).toBe("Hello world");
  });

  it("converts br to newline", () => {
    expect(assembleNoteContext("Line 1<br>Line 2")).toBe("Line 1\nLine 2");
  });

  it("decodes HTML entities", () => {
    expect(assembleNoteContext("A &amp; B &lt; C &gt; D")).toBe("A & B < C > D");
  });

  it("decodes quote entities", () => {
    expect(assembleNoteContext("&quot;hello&quot; &#39;world&#39;")).toBe(
      '"hello" \'world\'',
    );
  });

  it("collapses excessive newlines", () => {
    expect(assembleNoteContext("<p>A</p><p></p><p></p><p>B</p>")).toBe(
      "A\n\nB",
    );
  });
});

describe("markdownToBasicHtml", () => {
  it("converts bold text", () => {
    const html = markdownToBasicHtml("**bold**");
    expect(html).toContain("<strong>bold</strong>");
  });

  it("converts unordered list", () => {
    const html = markdownToBasicHtml("- item 1\n- item 2");
    expect(html).toContain("<li>item 1</li>");
    expect(html).toContain("<li>item 2</li>");
  });

  it("converts headings", () => {
    const html = markdownToBasicHtml("# Title");
    expect(html).toContain("<h1");
    expect(html).toContain("Title");
  });

  it("strips script tags (XSS prevention)", () => {
    const html = markdownToBasicHtml('<script>alert("xss")</script>');
    expect(html).not.toContain("<script");
    expect(html).not.toContain("alert");
  });

  it("strips event handler attributes", () => {
    const html = markdownToBasicHtml('<img src="x" onerror="alert(1)">');
    expect(html).not.toContain("onerror");
    expect(html).not.toContain("alert");
  });

  it("strips javascript: URLs", () => {
    const html = markdownToBasicHtml('[click](javascript:alert(1))');
    expect(html).not.toContain("javascript:");
  });

  it("preserves safe HTML from markdown", () => {
    const html = markdownToBasicHtml("**safe** and *italic*");
    expect(html).toContain("<strong>safe</strong>");
    expect(html).toContain("<em>italic</em>");
  });
});

describe("fetchCustomModels", () => {
  const originalFetch = globalThis.fetch;

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  it("appends /models to baseUrl and returns string ids", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        data: [
          { id: "qwen2.5-7b" },
          { id: "llama-3.1-8b" },
        ],
      }),
    }) as unknown as typeof fetch;

    const ids = await fetchCustomModels("http://127.0.0.1:8080/v1");
    expect(ids).toEqual(["qwen2.5-7b", "llama-3.1-8b"]);
    expect(globalThis.fetch).toHaveBeenCalledWith("http://127.0.0.1:8080/v1/models");
  });

  it("strips trailing slash before appending /models", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ data: [] }),
    }) as unknown as typeof fetch;

    await fetchCustomModels("http://127.0.0.1:8080/v1/");
    expect(globalThis.fetch).toHaveBeenCalledWith("http://127.0.0.1:8080/v1/models");
  });

  it("throws HTTP status on non-OK response", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 404,
    }) as unknown as typeof fetch;

    await expect(fetchCustomModels("http://x/v1")).rejects.toThrow("HTTP 404");
  });

  it("throws on unexpected response shape", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ models: [] }),
    }) as unknown as typeof fetch;

    await expect(fetchCustomModels("http://x/v1")).rejects.toThrow(
      "Unexpected response shape",
    );
  });

  it("filters out entries without a string id", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        data: [{ id: "a" }, { id: 42 }, { notId: "x" }, { id: "b" }],
      }),
    }) as unknown as typeof fetch;

    const ids = await fetchCustomModels("http://x/v1");
    expect(ids).toEqual(["a", "b"]);
  });
});
