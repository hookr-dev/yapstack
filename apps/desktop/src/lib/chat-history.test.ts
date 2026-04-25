/**
 * Pure-function tests for the chat-history assembler and the OpenAI
 * invariant guard. No DB, no LLM — fixtures are hand-rolled DbChatMessage
 * arrays and ChatCompletionMessageParam[] arrays.
 */
import { describe, it, expect } from "vitest";
import {
  assembleHistoryForRequest,
  ensureToolResultsFollowToolUse,
} from "./chat-history";
import type { DbChatMessage } from "./db";
import type { ChatCompletionMessageParam } from "openai/resources/chat/completions";

function row(overrides: Partial<DbChatMessage>): DbChatMessage {
  return {
    id: "row-id",
    context_key: "ctx",
    session_id: null,
    role: "user",
    content: "",
    action: null,
    created_at: "2026-04-25T10:00:00",
    tool_calls: null,
    send_id: "send-1",
    sequence: 0,
    tool_call_id: null,
    observation: null,
    status: null,
    ...overrides,
  };
}

describe("assembleHistoryForRequest", () => {
  it("emits a clean user-assistant pair from prose-only rows", () => {
    const rows: DbChatMessage[] = [
      row({ id: "u1", role: "user", content: "hi", sequence: 0 }),
      row({ id: "a1", role: "assistant", content: "hello!", sequence: 1 }),
    ];
    const out = assembleHistoryForRequest(rows);
    expect(out).toEqual([
      { role: "user", content: "hi" },
      { role: "assistant", content: "hello!" },
    ]);
  });

  it("rebuilds assistant.tool_calls + tool messages with matching IDs", () => {
    const rows: DbChatMessage[] = [
      row({ id: "u1", role: "user", content: "find sessions", sequence: 0 }),
      row({
        id: "a1",
        role: "assistant",
        content: "",
        sequence: 1,
        tool_calls: JSON.stringify([
          {
            id: "call_1",
            name: "search_sessions",
            arguments: '{"query":"meeting"}',
            label: "Sessions",
            status: "done",
          },
        ]),
      }),
      row({
        id: "t1",
        role: "tool",
        content: "Matched 3 sessions",
        sequence: 2,
        tool_call_id: "call_1",
        status: "done",
      }),
      row({ id: "a2", role: "assistant", content: "Here are 3 matches.", sequence: 3 }),
    ];
    const out = assembleHistoryForRequest(rows);
    expect(out).toHaveLength(4);
    expect(out[0]).toEqual({ role: "user", content: "find sessions" });
    expect(out[1]).toMatchObject({
      role: "assistant",
      content: null,
      tool_calls: [
        {
          id: "call_1",
          type: "function",
          function: { name: "search_sessions", arguments: '{"query":"meeting"}' },
        },
      ],
    });
    expect(out[2]).toEqual({
      role: "tool",
      tool_call_id: "call_1",
      content: "Matched 3 sessions",
    });
    expect(out[3]).toEqual({ role: "assistant", content: "Here are 3 matches." });
  });

  it("falls back to text-only for legacy v13 rows missing call IDs", () => {
    const rows: DbChatMessage[] = [
      row({ id: "u1", role: "user", content: "old chat", sequence: 0 }),
      row({
        id: "a1",
        role: "assistant",
        content: "Did the thing.",
        sequence: 1,
        // Pre-v14 shape: no `id`, no `arguments`. Assembler skips replay.
        tool_calls: JSON.stringify([
          { name: "update_title", label: "Title", status: "done", detail: "Renamed" },
        ]),
      }),
    ];
    const out = assembleHistoryForRequest(rows);
    expect(out).toEqual([
      { role: "user", content: "old chat" },
      { role: "assistant", content: "Did the thing." },
    ]);
  });

  it("drops malformed rows that have neither prose nor v14 tool_calls", () => {
    const rows: DbChatMessage[] = [
      row({ id: "u1", role: "user", content: "hi", sequence: 0 }),
      row({ id: "a1", role: "assistant", content: "", sequence: 1, tool_calls: null }),
    ];
    const out = assembleHistoryForRequest(rows);
    expect(out).toEqual([{ role: "user", content: "hi" }]);
  });

  it("drops tool rows missing tool_call_id", () => {
    const rows: DbChatMessage[] = [
      row({ id: "u1", role: "user", content: "hi", sequence: 0 }),
      row({
        id: "t1",
        role: "tool",
        content: "orphan",
        sequence: 1,
        tool_call_id: null,
      }),
    ];
    const out = assembleHistoryForRequest(rows);
    expect(out).toEqual([{ role: "user", content: "hi" }]);
  });
});

describe("ensureToolResultsFollowToolUse", () => {
  function asst(toolCalls: { id: string; name: string }[]): ChatCompletionMessageParam {
    return {
      role: "assistant",
      content: null,
      tool_calls: toolCalls.map((c) => ({
        id: c.id,
        type: "function" as const,
        function: { name: c.name, arguments: "{}" },
      })),
    };
  }
  function tool(id: string, content = "ok"): ChatCompletionMessageParam {
    return { role: "tool", tool_call_id: id, content };
  }

  it("passes through valid sequences unchanged", () => {
    const input: ChatCompletionMessageParam[] = [
      { role: "user", content: "go" },
      asst([{ id: "c1", name: "f" }]),
      tool("c1"),
      { role: "assistant", content: "done" },
    ];
    expect(ensureToolResultsFollowToolUse(input)).toEqual(input);
  });

  it("reorders out-of-order tool messages", () => {
    const input: ChatCompletionMessageParam[] = [
      asst([
        { id: "c1", name: "f" },
        { id: "c2", name: "g" },
      ]),
      tool("c2", "second result"),
      tool("c1", "first result"),
    ];
    const out = ensureToolResultsFollowToolUse(input);
    expect(out[1]).toEqual(tool("c1", "first result"));
    expect(out[2]).toEqual(tool("c2", "second result"));
  });

  it("inserts placeholder for missing tool result", () => {
    const input: ChatCompletionMessageParam[] = [
      asst([{ id: "c1", name: "f" }]),
    ];
    const out = ensureToolResultsFollowToolUse(input);
    expect(out).toHaveLength(2);
    expect(out[1]).toEqual({
      role: "tool",
      tool_call_id: "c1",
      content: "(result missing)",
    });
  });

  it("drops orphan tool messages with no preceding assistant call", () => {
    const input: ChatCompletionMessageParam[] = [
      { role: "user", content: "hi" },
      tool("c1", "orphan"),
      { role: "assistant", content: "answer" },
    ];
    const out = ensureToolResultsFollowToolUse(input);
    expect(out).toEqual([
      { role: "user", content: "hi" },
      { role: "assistant", content: "answer" },
    ]);
  });

  it("handles a multi-round conversation end-to-end", () => {
    const input: ChatCompletionMessageParam[] = [
      { role: "user", content: "find and use" },
      asst([{ id: "c1", name: "search" }]),
      tool("c1", "found 3"),
      asst([{ id: "c2", name: "expand" }]),
      tool("c2", "expanded"),
      { role: "assistant", content: "summary" },
    ];
    expect(ensureToolResultsFollowToolUse(input)).toEqual(input);
  });
});
