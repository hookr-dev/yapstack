import OpenAI from "openai";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import type {
  ChatCompletionMessageParam,
  ChatCompletionTool,
} from "openai/resources/chat/completions";
import { marked } from "marked";
import DOMPurify from "dompurify";
import type { DbSegment } from "./db";
import type { SessionWithNote } from "./db";
import type { DbDictationHistory } from "./db";
import type { ToolCallResult } from "./ai-tools";

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
}

export interface AISettings {
  activeProvider: AIProvider;
  providers: Record<AIProvider, AIProviderConfig>;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  action?: AIActionType;
  isStreaming?: boolean;
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
      model: "gpt-4o-mini",
      baseUrl: "https://api.openai.com/v1",
    },
    openrouter: {
      apiKey: "",
      model: "anthropic/claude-sonnet-4",
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
    { id: "gpt-4o", label: "GPT-4o", recommended: true },
    { id: "gpt-4o-mini", label: "GPT-4o Mini" },
    { id: "o3-mini", label: "o3 Mini" },
    { id: "gpt-4-turbo", label: "GPT-4 Turbo" },
  ],
  openrouter: [
    { id: "anthropic/claude-sonnet-4", label: "Claude Sonnet 4", recommended: true },
    { id: "anthropic/claude-opus-4", label: "Claude Opus 4" },
    { id: "anthropic/claude-haiku-3.5", label: "Claude Haiku 3.5" },
    { id: "anthropic/claude-sonnet-3.5", label: "Claude Sonnet 3.5" },
    { id: "openai/gpt-4o", label: "GPT-4o" },
    { id: "openai/gpt-4o-mini", label: "GPT-4o Mini" },
    { id: "google/gemini-2.0-flash", label: "Gemini 2.0 Flash" },
  ],
};

export function getModelsForProvider(provider: AIProvider): ModelOption[] | null {
  return MODEL_CATALOG[provider] ?? null;
}

export interface GroupedModels {
  provider: AIProvider;
  providerLabel: string;
  models: (ModelOption & { available: boolean })[];
}

const PROVIDER_DISPLAY: Record<string, string> = {
  openai: "OpenAI",
  openrouter: "OpenRouter",
};

export function getAllModelsGrouped(activeProvider: AIProvider): GroupedModels[] {
  const groups: GroupedModels[] = [];
  const providers = Object.keys(MODEL_CATALOG) as AIProvider[];
  const sorted = [activeProvider, ...providers.filter((p) => p !== activeProvider)];
  for (const p of sorted) {
    const models = MODEL_CATALOG[p];
    if (!models) continue;
    groups.push({
      provider: p,
      providerLabel: PROVIDER_DISPLAY[p] ?? p,
      models: models.map((m) => ({ ...m, available: p === activeProvider })),
    });
  }
  return groups;
}

// ----- Client -----

export function createAIClient(settings: AISettings): OpenAI {
  const config = settings.providers[settings.activeProvider];

  const headers: Record<string, string> = {};
  if (settings.activeProvider === "openrouter") {
    headers["HTTP-Referer"] = "https://yapstack.app";
    headers["X-Title"] = "YapStack";
  }

  // Local OpenAI-compatible servers (llama.cpp, LM Studio, Ollama) don't require a key,
  // but the OpenAI SDK refuses to construct without one. Substitute a placeholder.
  const apiKey =
    settings.activeProvider === "custom" && !config.apiKey
      ? "sk-no-key-required"
      : config.apiKey;

  return new OpenAI({
    apiKey,
    baseURL: config.baseUrl,
    dangerouslyAllowBrowser: true,
    defaultHeaders: Object.keys(headers).length > 0 ? headers : undefined,
    fetch: tauriFetch,
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
      const speakerPrefix =
        s.speaker_id != null
          ? ` (${speakerNames?.[s.speaker_id] ?? `Speaker ${s.speaker_id + 1}`})`
          : "";
      return `[seg:${s.id} ${ts}]${speakerPrefix} ${s.text}`;
    })
    .join("\n");
}

/// True when any segment carries a non-null `speaker_id` — i.e. the
/// transcript came from a diarized session. Used by the system-prompt
/// builder to decide whether to include speaker guidance.
export function transcriptHasSpeakers(segments: DbSegment[]): boolean {
  return segments.some((s) => s.speaker_id != null);
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

// ----- Message Building -----

// ----- Multi-session context -----

export function assembleMultiSessionContext(
  sessions: SessionWithNote[],
  includeNotes: boolean,
): string {
  return sessions
    .map((s) => {
      const date = new Date(s.createdAt).toLocaleDateString();
      let block = `- **${s.title || "Untitled"}** (${date})`;
      if (includeNotes && s.noteContent) {
        const plain = assembleNoteContext(s.noteContent);
        if (plain) {
          block += "\n  Notes:\n" + plain.split("\n").map((l) => `    ${l}`).join("\n");
        }
      }
      return block;
    })
    .join("\n\n");
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

    // Text content
    if (delta?.content) {
      yield { type: "token", content: delta.content };
    }

    // Tool call deltas
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

  // Emit accumulated tool calls
  if (toolCallMap.size > 0) {
    const calls: ToolCallResult[] = [];
    for (const [, tc] of toolCallMap) {
      try {
        const parsed = JSON.parse(tc.arguments) as Record<string, unknown>;
        calls.push({ id: tc.id, name: tc.name, arguments: parsed });
      } catch {
        // Skip malformed tool call arguments
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
      max_tokens: 1,
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
