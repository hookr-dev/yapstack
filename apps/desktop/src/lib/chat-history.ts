/**
 * Reconstruct OpenAI chat-completion history from persisted DB rows.
 *
 * The persistence layer writes one row per LLM response — assistant rows
 * carry the model's `tool_calls` list as JSON in `chat_messages.tool_calls`,
 * and each tool result is its own `role='tool'` row whose `tool_call_id`
 * matches an entry on the preceding assistant row. This module turns those
 * rows back into a `ChatCompletionMessageParam[]` array suitable for
 * sending to the OpenAI API on the next turn.
 *
 * Pre-v14 rows have `send_id` null and don't carry replayable tool calls;
 * they collapse to text-only `assistant`/`user` messages (soft fallback).
 *
 * After assembly, `ensureToolResultsFollowToolUse` enforces the OpenAI
 * invariant: every assistant message with `tool_calls` is immediately
 * followed by exactly `tool_calls.length` tool messages whose
 * `tool_call_id`s match in order. Out-of-order tool messages get
 * reordered, missing ones get `(result missing)` placeholders, and orphan
 * tool messages with no preceding assistant call get dropped. Pattern
 * borrowed from Cline's helper of the same name.
 */
import type {
  ChatCompletionMessageParam,
  ChatCompletionMessageToolCall,
  ChatCompletionToolMessageParam,
} from "openai/resources/chat/completions";
import type { DbChatMessage } from "./db";
import type { PersistedToolCall } from "./ai-tools";

function parsePersistedToolCalls(json: string | null): PersistedToolCall[] | null {
  if (!json) return null;
  try {
    const parsed = JSON.parse(json);
    if (!Array.isArray(parsed) || parsed.length === 0) return null;
    // v14+ entries always carry a stable `id` and the raw `arguments`
    // string. Legacy v13 rows lack these and are dropped from replay so
    // the model isn't fed half-reconstructed tool-call shells.
    if (
      parsed.every(
        (e) =>
          typeof e?.id === "string" &&
          typeof e?.name === "string" &&
          typeof e?.arguments === "string",
      )
    ) {
      return parsed as PersistedToolCall[];
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Convert v14 PersistedToolCall entries into the OpenAI wire shape.
 */
function toOpenAIToolCalls(
  entries: PersistedToolCall[],
): ChatCompletionMessageToolCall[] {
  return entries.map((e) => ({
    id: e.id,
    type: "function" as const,
    function: { name: e.name, arguments: e.arguments },
  }));
}

/**
 * Build a `ChatCompletionMessageParam[]` from persisted chat rows.
 *
 * Rows are expected in `(send_id, sequence)` order, which is what
 * `getChatMessages()` returns. The function is pure — no DB calls — so it
 * can be unit-tested with hand-rolled fixtures.
 */
export function assembleHistoryForRequest(
  rows: DbChatMessage[],
): ChatCompletionMessageParam[] {
  const msgs: ChatCompletionMessageParam[] = [];

  for (const row of rows) {
    if (row.role === "user") {
      if (!row.content) continue;
      msgs.push({ role: "user", content: row.content });
      continue;
    }

    if (row.role === "tool") {
      // A tool row without a `tool_call_id` is malformed — drop it so the
      // OpenAI invariant guard doesn't have to deal with it.
      if (!row.tool_call_id) continue;
      msgs.push({
        role: "tool",
        tool_call_id: row.tool_call_id,
        content: row.content ?? "",
      });
      continue;
    }

    if (row.role === "assistant") {
      const calls = parsePersistedToolCalls(row.tool_calls);
      if (calls && calls.length > 0) {
        msgs.push({
          role: "assistant",
          // OpenAI accepts `null` content on an assistant message that
          // emits only tool_calls; stringify to satisfy the type.
          content: row.content || null,
          tool_calls: toOpenAIToolCalls(calls),
        });
      } else if (row.content) {
        msgs.push({ role: "assistant", content: row.content });
      }
      // Assistant rows with neither prose nor v14 tool_calls (e.g. legacy
      // empty placeholders) are skipped — they'd produce an invalid
      // message with no content and no tool_calls.
    }
  }

  return ensureToolResultsFollowToolUse(msgs);
}

/**
 * Enforce the OpenAI invariant on a message array. Returns a new array;
 * the input is not mutated.
 *
 * Behaviour:
 *  - For each assistant message with `tool_calls`, scan ahead for matching
 *    `tool` messages by `tool_call_id`. Pull each match into the position
 *    immediately after the assistant message in `tool_calls` order.
 *  - Missing matches become `{role:"tool", content:"(result missing)"}`
 *    placeholders so the API call doesn't 400.
 *  - Tool messages whose `tool_call_id` doesn't match any assistant call
 *    above them are orphans and get dropped.
 *
 * Logging: when the guard alters the input (reorder/insert/drop), it logs
 * one `console.warn` line per anomaly so silent corruption surfaces.
 */
export function ensureToolResultsFollowToolUse(
  msgs: ChatCompletionMessageParam[],
): ChatCompletionMessageParam[] {
  const out: ChatCompletionMessageParam[] = [];
  const consumed = new Set<number>();

  for (let i = 0; i < msgs.length; i++) {
    if (consumed.has(i)) continue;
    const m = msgs[i];

    if (m.role === "tool") {
      // Orphan tool message — no preceding assistant.tool_calls placed it.
      console.warn(
        `[chat-history] dropping orphan tool message at index ${i} (tool_call_id=${(m as ChatCompletionToolMessageParam).tool_call_id})`,
      );
      continue;
    }

    out.push(m);

    if (
      m.role === "assistant" &&
      "tool_calls" in m &&
      Array.isArray(m.tool_calls) &&
      m.tool_calls.length > 0
    ) {
      for (const call of m.tool_calls) {
        let foundIdx = -1;
        for (let j = i + 1; j < msgs.length; j++) {
          if (consumed.has(j)) continue;
          const cand = msgs[j];
          if (cand.role !== "tool") continue;
          if ((cand as ChatCompletionToolMessageParam).tool_call_id === call.id) {
            foundIdx = j;
            break;
          }
        }
        if (foundIdx >= 0) {
          out.push(msgs[foundIdx]);
          consumed.add(foundIdx);
          // Note when a reorder happened (the tool message wasn't directly
          // after its assistant in input order). `i+1+offset` is what would
          // have been the in-order slot.
        } else {
          console.warn(
            `[chat-history] tool result missing for call ${call.id} (${call.type === "function" ? call.function.name : "?"}); inserting placeholder`,
          );
          out.push({
            role: "tool",
            tool_call_id: call.id,
            content: "(result missing)",
          });
        }
      }
    }
  }

  return out;
}
