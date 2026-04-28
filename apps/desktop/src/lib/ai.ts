import OpenAI from "openai";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import type {
  ChatCompletionMessageParam,
  ChatCompletionTool,
} from "openai/resources/chat/completions";
import { marked } from "marked";
import DOMPurify from "dompurify";
import type { DbSegment } from "./db";
import type { DbDictationHistory } from "./db";
import type { ToolCallResult } from "./ai-tools";
import type { FolderTreeNode } from "./folder-tree";

// ----- ChatContext -----

export type ChatContext =
  | { type: "session"; sessionId: string }
  | { type: "folder"; folderId: string }
  | { type: "all" }
  | { type: "pinned" }
  | { type: "dictation" };

export function chatContextKey(ctx: ChatContext): string {
  switch (ctx.type) {
    case "session":
      return ctx.sessionId;
    case "folder":
      return `folder:${ctx.folderId}`;
    case "all":
      return "global";
    case "pinned":
      return "pinned";
    case "dictation":
      return "dictation";
  }
}

// ----- Types -----

export type AIProvider = "openai" | "openrouter" | "custom";

export interface AIProviderConfig {
  apiKey: string;
  model: string;
  baseUrl: string;
  // Populated by `fetchCustomModels` for OpenAI-compatible local/remote
  // servers. Persisted so both Settings and the chat model-picker read
  // from the same source. Undefined until the user fetches at least once.
  fetchedModels?: string[];
}

export interface AISettings {
  activeProvider: AIProvider;
  providers: Record<AIProvider, AIProviderConfig>;
}

export type ToolExecutionStatus = "running" | "done" | "error";

export interface ToolExecution {
  name: string;
  label: string;
  detail?: string;
  status: ToolExecutionStatus;
  /** True if the user reverted this call via Undo. Renderer applies a
   * grayed/strike-through style; the call still appears in chat history. */
  undone?: boolean;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  action?: AIActionType;
  isStreaming?: boolean;
  toolExecutions?: ToolExecution[];
}

export interface FileAttachment {
  name: string;
  content: string;
}

/** Any action ID. Built-in: "summarize" | "key-points" | "action-items" | "meeting-minutes" | "general" */
export type AIActionType = string;

// ----- Defaults -----

export const DEFAULT_AI_SETTINGS: AISettings = {
  activeProvider: "openai",
  providers: {
    openai: {
      apiKey: "",
      model: "gpt-5.4-mini",
      baseUrl: "https://api.openai.com/v1",
    },
    openrouter: {
      apiKey: "",
      model: "anthropic/claude-haiku-4.5",
      baseUrl: "https://openrouter.ai/api/v1",
    },
    custom: {
      apiKey: "",
      model: "",
      baseUrl: "http://127.0.0.1:8080/v1",
    },
  },
};

// ----- Model Catalog -----

export interface ModelOption {
  id: string;
  label: string;
  recommended?: boolean;
}

export const MODEL_CATALOG: Partial<Record<AIProvider, ModelOption[]>> = {
  openai: [
    // GPT-5.4 — current flagship family
    { id: "gpt-5.4-mini", label: "GPT-5.4 Mini", recommended: true },
    { id: "gpt-5.4", label: "GPT-5.4" },
    { id: "gpt-5.4-nano", label: "GPT-5.4 Nano" },
    { id: "gpt-5.4-pro", label: "GPT-5.4 Pro" },
    // GPT-5.2 — reasoning/thinking
    { id: "gpt-5.2", label: "GPT-5.2 (thinking)" },
    // GPT-4.1 — 1M-token long context
    { id: "gpt-4.1", label: "GPT-4.1" },
    { id: "gpt-4.1-mini", label: "GPT-4.1 Mini" },
    { id: "gpt-4.1-nano", label: "GPT-4.1 Nano" },
    // o-series reasoning
    { id: "o4-mini", label: "o4 Mini" },
    { id: "o3", label: "o3" },
    // Legacy — still live, kept for existing installs
    { id: "gpt-4o", label: "GPT-4o" },
    { id: "gpt-4o-mini", label: "GPT-4o Mini" },
  ],
  openrouter: [
    // Anthropic Claude — best tool-calling quality
    { id: "anthropic/claude-haiku-4.5", label: "Claude Haiku 4.5", recommended: true },
    { id: "anthropic/claude-sonnet-4.5", label: "Claude Sonnet 4.5" },
    // OpenAI via OpenRouter
    { id: "openai/gpt-5.4", label: "GPT-5.4" },
    { id: "openai/gpt-5.4-mini", label: "GPT-5.4 Mini" },
    { id: "openai/gpt-5.4-nano", label: "GPT-5.4 Nano" },
    { id: "openai/gpt-5.2", label: "GPT-5.2 (thinking)" },
    // Google Gemini
    { id: "google/gemini-3.1-pro", label: "Gemini 3.1 Pro" },
    { id: "google/gemini-3.1-flash-lite", label: "Gemini 3.1 Flash Lite" },
    // Budget frontier-class option
    { id: "deepseek/deepseek-v3.2", label: "DeepSeek V3.2" },
  ],
};

export function getModelsForProvider(provider: AIProvider): ModelOption[] | null {
  return MODEL_CATALOG[provider] ?? null;
}

export interface GroupedModels {
  provider: AIProvider;
  providerLabel: string;
  models: ModelOption[];
}

const PROVIDER_DISPLAY: Record<string, string> = {
  openai: "OpenAI",
  openrouter: "OpenRouter",
  custom: "Custom",
};

/// Returns model groups to display for the *active* provider only. For
/// built-in providers this reads `MODEL_CATALOG`; for `custom` it reads
/// the persisted `config.fetchedModels`. Cross-provider models are
/// deliberately omitted — the old greyed-out UX confused users.
export function getAllModelsGrouped(
  activeProvider: AIProvider,
  activeConfig?: AIProviderConfig,
): GroupedModels[] {
  if (activeProvider === "custom") {
    const fetched = activeConfig?.fetchedModels ?? [];
    if (fetched.length === 0) return [];
    return [
      {
        provider: "custom",
        providerLabel: PROVIDER_DISPLAY.custom,
        models: fetched.map((id) => ({ id, label: id })),
      },
    ];
  }
  const models = MODEL_CATALOG[activeProvider];
  if (!models) return [];
  return [
    {
      provider: activeProvider,
      providerLabel: PROVIDER_DISPLAY[activeProvider] ?? activeProvider,
      models,
    },
  ];
}

// ----- Client -----

// The OpenAI SDK passes a `Headers` instance to its custom `fetch`. Tauri's
// `plugin-http` then runs `new Request(input, init)` against that same init,
// and on macOS WKWebView this loses the `Authorization` header on cross-origin
// requests — OpenRouter then responds 401 "Missing Authentication header".
// Converting headers to a plain object before invoking `tauriFetch` sidesteps
// the Request-constructor path entirely.
async function tauriFetchAdapter(
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  if (!init?.headers) return tauriFetch(input as string, init);
  const plain: Record<string, string> = {};
  if (init.headers instanceof Headers) {
    init.headers.forEach((value, key) => {
      plain[key] = value;
    });
  } else if (Array.isArray(init.headers)) {
    for (const [key, value] of init.headers) plain[key] = value;
  } else {
    Object.assign(plain, init.headers);
  }
  return tauriFetch(input as string, { ...init, headers: plain });
}

export function createAIClient(settings: AISettings): OpenAI {
  const config = settings.providers[settings.activeProvider];

  const headers: Record<string, string> = {};
  if (settings.activeProvider === "openrouter") {
    headers["HTTP-Referer"] = "https://yapstack.app";
    headers["X-Title"] = "YapStack";
  }

  // Trim whitespace — pasted keys frequently carry a trailing newline, which
  // produces an empty Bearer token after the SDK's `Bearer ${apiKey}` template
  // is split on the first space by some servers.
  const trimmedKey = config.apiKey.trim();

  // Local OpenAI-compatible servers (llama.cpp, LM Studio, Ollama) don't require a key,
  // but the OpenAI SDK refuses to construct without one. Substitute a placeholder.
  const apiKey =
    settings.activeProvider === "custom" && !trimmedKey
      ? "sk-no-key-required"
      : trimmedKey;

  return new OpenAI({
    apiKey,
    baseURL: config.baseUrl,
    dangerouslyAllowBrowser: true,
    defaultHeaders: Object.keys(headers).length > 0 ? headers : undefined,
    fetch: tauriFetchAdapter,
  });
}

export async function fetchCustomModels(baseUrl: string): Promise<string[]> {
  const url = baseUrl.replace(/\/$/, "") + "/models";
  const res = await tauriFetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const json = (await res.json()) as { data?: Array<{ id?: unknown }> };
  if (!Array.isArray(json.data)) throw new Error("Unexpected response shape");
  return json.data
    .map((m) => (typeof m.id === "string" ? m.id : null))
    .filter((id): id is string => !!id);
}

export function getActiveConfig(settings: AISettings): AIProviderConfig {
  return settings.providers[settings.activeProvider];
}

/**
 * A provider is "configured" (usable for AI features) when the server can be
 * reached and a model is named. Custom providers (local llama.cpp / LM Studio /
 * Ollama) accept a blank API key — an empty key must not count as "not set up"
 * for them, or dictation silently skips its AI cleanup step.
 */
export function isAIConfigured(settings: AISettings): boolean {
  const config = getActiveConfig(settings);
  if (settings.activeProvider === "custom") {
    return !!config.baseUrl && !!config.model;
  }
  return !!config.apiKey;
}

// ----- Context Assembly -----

/**
 * Speaker label for a segment in transcript text rendered for the LLM.
 *
 * The convention is the same everywhere a segment is shown to the model
 * (single-session transcript context, `get_session_context` tool output,
 * `search_sessions` snippets). The model relies on this label — without
 * it, statements from other speakers look indistinguishable from things
 * the user said.
 *
 *  - `Mic`  → "You" (the human running the app, captured via microphone).
 *  - `System` with `speaker_id` set → custom name from `speakerNames` if
 *    present, else `Speaker N` (1-indexed for human-friendly numbering).
 *    System audio is everyone else: people on a video call, podcast
 *    audio, etc.
 *  - `System` without diarization → "Other" (we know it isn't the user
 *    but we have no further identification).
 *
 * Returning a flat string (not optional) means callers can always wrap
 * it in `(...)` without conditional formatting.
 */
export function formatSegmentSpeaker(
  seg: DbSegment,
  speakerNames?: Record<number, string>,
): string {
  if (seg.source === "Mic") return "You";
  if (seg.speaker_id != null) {
    return speakerNames?.[seg.speaker_id] ?? `Speaker ${seg.speaker_id + 1}`;
  }
  return "Other";
}

export function assembleTranscriptContext(
  segments: DbSegment[],
  speakerNames?: Record<number, string>,
): string {
  return segments
    .filter((s) => s.hidden !== 1 && !s.deleted_at)
    .map((s) => {
      const mins = Math.floor(s.audio_offset_seconds / 60);
      const secs = Math.floor(s.audio_offset_seconds % 60);
      const ts = `${mins}:${secs.toString().padStart(2, "0")}`;
      const label = formatSegmentSpeaker(s, speakerNames);
      return `[seg:${s.id} ${ts}] (${label}) ${s.text}`;
    })
    .join("\n");
}

/**
 * True for any transcript with at least one segment — speaker labels are
 * now applied universally (see `formatSegmentSpeaker`), so the
 * SPEAKER_INSTRUCTION prompt always applies when a transcript is in
 * scope. Kept as a function (not inlined) so callers stay readable and
 * we have one place to flip if the convention ever needs to be gated
 * again (e.g. for an engine that produces no source-tagged segments).
 */
export function transcriptHasSpeakers(segments: DbSegment[]): boolean {
  return segments.length > 0;
}

export function assembleNoteContext(noteHtml: string): string {
  // Strip HTML tags to get plain text
  return noteHtml
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/p>/gi, "\n")
    .replace(/<\/h[1-6]>/gi, "\n")
    .replace(/<\/li>/gi, "\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

// ----- Folder Tree Context -----

export function assembleFolderTreeContext(tree: FolderTreeNode[]): string {
  function renderNode(node: FolderTreeNode, depth: number): string {
    const indent = "  ".repeat(depth);
    const desc = node.folder.description
      ? ` — "${node.folder.description}"`
      : "";
    let line = `${indent}- [${node.folder.id}] ${node.folder.name}${desc}`;
    for (const child of node.children) {
      line += "\n" + renderNode(child, depth + 1);
    }
    return line;
  }
  return tree.map((root) => renderNode(root, 0)).join("\n");
}

// ----- Multi-session context -----

/**
 * Compact, retrieval-friendly listing of session candidates. Used by
 * multi-session Chats so the LLM sees a candidate list (id + title + date
 * + folder path) and decides which sessions to expand via the
 * `get_session_context` tool, instead of being handed every session's
 * notes/transcript up front.
 */
export function assembleSessionCandidates(
  candidates: { id: string; title: string; date: string; folderPath: string | null }[],
): string {
  if (candidates.length === 0) return "";
  const lines = candidates.map((c) => {
    const date = c.date ? new Date(c.date).toLocaleDateString() : "";
    const folder = c.folderPath ? ` folder="${c.folderPath}"` : "";
    return `- session_id=${c.id} title="${c.title || "Untitled"}" date=${date}${folder}`;
  });
  return lines.join("\n");
}

// ----- Dictation Context Assembly -----

export function assembleDictationContext(entries: DbDictationHistory[]): string {
  return entries
    .map((e) => {
      const date = new Date(e.created_at);
      const dateStr = date.toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
      });
      const timeStr = date.toLocaleTimeString(undefined, {
        hour: "numeric",
        minute: "2-digit",
      });
      const dur = e.wav_duration_seconds
        ? `${Math.round(e.wav_duration_seconds)}s`
        : "";
      const meta = [dateStr, timeStr, dur].filter(Boolean).join(" · ");

      let block = `- **${e.slot_name}** (${meta})`;

      const inputDiffers = e.ai_enabled && e.input_text !== e.output_text;
      if (inputDiffers) {
        block += `\n  Input: "${e.input_text}"`;
        block += `\n  Output: "${e.output_text}"`;
      } else {
        block += `\n  Text: "${e.output_text || e.input_text}"`;
      }

      if (e.ai_enabled && e.ai_prompt) {
        block += `\n  AI prompt: "${e.ai_prompt}"`;
      }
      block += `\n  Action: ${e.output_action}`;

      return block;
    })
    .join("\n\n");
}

// ----- Streaming -----

export async function* streamChat(
  client: OpenAI,
  model: string,
  messages: ChatCompletionMessageParam[],
  signal?: AbortSignal,
): AsyncGenerator<string> {
  const stream = await client.chat.completions.create(
    {
      model,
      messages,
      stream: true,
    },
    { signal },
  );

  for await (const chunk of stream) {
    const delta = chunk.choices[0]?.delta?.content;
    if (delta) {
      yield delta;
    }
  }
}

// ----- Streaming with Tool Calls -----

export type StreamEvent =
  | { type: "token"; content: string }
  | { type: "tool_calls"; calls: ToolCallResult[] }
  | { type: "done" };

export async function* streamChatWithTools(
  client: OpenAI,
  model: string,
  messages: ChatCompletionMessageParam[],
  tools: ChatCompletionTool[],
  signal?: AbortSignal,
): AsyncGenerator<StreamEvent> {
  const stream = await client.chat.completions.create(
    {
      model,
      messages,
      tools: tools.length > 0 ? tools : undefined,
      tool_choice: tools.length > 0 ? "auto" : undefined,
      stream: true,
    },
    { signal },
  );

  // Accumulate tool calls across chunks
  const toolCallMap = new Map<
    number,
    { id: string; name: string; arguments: string }
  >();

  for await (const chunk of stream) {
    const choice = chunk.choices[0];
    if (!choice) continue;

    const delta = choice.delta;

    if (delta?.content) {
      yield { type: "token", content: delta.content };
    }

    if (delta?.tool_calls) {
      for (const tc of delta.tool_calls) {
        const existing = toolCallMap.get(tc.index);
        if (existing) {
          if (tc.function?.arguments) {
            existing.arguments += tc.function.arguments;
          }
        } else {
          toolCallMap.set(tc.index, {
            id: tc.id ?? "",
            name: tc.function?.name ?? "",
            arguments: tc.function?.arguments ?? "",
          });
        }
      }
    }
  }

  // Emit accumulated tool calls. If a parse fails we still emit the call so
  // the orchestrator can produce a tool-error result back to the model
  // (every tool_call must have a matching tool_result, otherwise
  // the next turn errors out).
  if (toolCallMap.size > 0) {
    const calls: ToolCallResult[] = [];
    for (const [, tc] of toolCallMap) {
      try {
        const parsed = JSON.parse(tc.arguments) as Record<string, unknown>;
        calls.push({ id: tc.id, name: tc.name, arguments: parsed });
      } catch (e) {
        console.warn(
          `[ai] tool ${tc.name} returned malformed JSON arguments: ${
            e instanceof Error ? e.message : String(e)
          }`,
        );
        calls.push({
          id: tc.id,
          name: tc.name,
          arguments: { __parseError: tc.arguments },
        });
      }
    }
    if (calls.length > 0) {
      yield { type: "tool_calls", calls };
    }
  }

  yield { type: "done" };
}

// ----- Connection Test -----

export async function testConnection(
  settings: AISettings,
): Promise<{ ok: boolean; error?: string }> {
  try {
    const client = createAIClient(settings);
    const config = getActiveConfig(settings);
    await client.chat.completions.create({
      model: config.model,
      messages: [{ role: "user", content: "Hi" }],
    });
    return { ok: true };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

// ----- Markdown to HTML -----

export function markdownToBasicHtml(md: string): string {
  const raw = marked.parse(md, { async: false }) as string;
  return DOMPurify.sanitize(raw);
}
