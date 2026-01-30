import type { ChatCompletionTool } from "openai/resources/chat/completions";
import {
  updateSessionTitle,
  saveNote,
  togglePin,
  getSession,
} from "./db";
import type { DbNote, DbSegment } from "./db";
import { markdownToBasicHtml } from "./ai";
import { formatTime } from "./utils";

// --- Core types ---

export interface ToolCallResult {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

export interface ExecutedTool {
  name: string;
  label: string;
  detail: string;
  undoData?: unknown;
}

export interface ToolContext {
  sessionId: string;
  currentTitle: string;
  currentNote: DbNote | null;
  isPinned: boolean;
  segments?: DbSegment[];
}

// --- Modular tool definition ---

export type ToolEffect = "session-meta" | "notes";

export interface ToolDefinition {
  schema: ChatCompletionTool;
  execute: (
    args: Record<string, unknown>,
    ctx: ToolContext,
  ) => Promise<ExecutedTool | null>;
  undo: (undoData: unknown, ctx: { sessionId: string }) => Promise<void>;
  affects?: ToolEffect[];
}

// --- Citation conversion ---

const CITE_REGEX = /\[\[seg:([a-zA-Z0-9_-]+)\]\]/g;

/**
 * Replace [[seg:ID]] text citations with <span data-segment-ref> HTML nodes
 * that the Tiptap SegmentReference extension can render as interactive pills.
 */
export function convertCitationsToSegmentRefs(
  html: string,
  segments: DbSegment[],
): string {
  return html.replace(CITE_REGEX, (_match, segId: string) => {
    const segment = segments.find((s) => s.id === segId);
    const ts = segment
      ? formatTime(Math.max(0, segment.audio_offset_seconds))
      : segId.slice(0, 8);
    const offset = segment ? segment.audio_offset_seconds : 0;
    return `<span data-segment-ref="" data-segment-id="${segId}" data-timestamp="${ts}" data-offset="${offset}">${ts}</span>`;
  });
}

// --- Registry ---

const toolRegistry = new Map<string, ToolDefinition>();

export function registerTool(def: ToolDefinition): void {
  const name =
    def.schema.type === "function" ? def.schema.function.name : undefined;
  if (name) toolRegistry.set(name, def);
}

export function getRegisteredTools(): ChatCompletionTool[] {
  return Array.from(toolRegistry.values()).map((t) => t.schema);
}

export function getToolsForContext(isSessionContext: boolean): ChatCompletionTool[] {
  return isSessionContext ? getRegisteredTools() : [];
}

export function getToolsById(toolIds: string[]): ChatCompletionTool[] {
  return toolIds
    .map((id) => toolRegistry.get(id))
    .filter((def): def is ToolDefinition => def !== undefined)
    .map((def) => def.schema);
}

export function getToolEffects(toolNames: string[]): Set<ToolEffect> {
  const effects = new Set<ToolEffect>();
  for (const name of toolNames) {
    const def = toolRegistry.get(name);
    if (def?.affects) for (const e of def.affects) effects.add(e);
  }
  return effects;
}

export async function executeTool(
  name: string,
  args: Record<string, unknown>,
  ctx: ToolContext,
): Promise<ExecutedTool | null> {
  const def = toolRegistry.get(name);
  if (!def) throw new Error(`Unknown tool: ${name}`);
  return def.execute(args, ctx);
}

export async function undoToolCalls(
  executed: ExecutedTool[],
  ctx: { sessionId: string },
): Promise<void> {
  for (const tool of [...executed].reverse()) {
    const def = toolRegistry.get(tool.name);
    if (def?.undo && tool.undoData !== undefined) {
      await def.undo(tool.undoData, ctx);
    }
  }
}

// --- Tool: update_title ---

registerTool({
  affects: ["session-meta"],
  schema: {
    type: "function",
    function: {
      name: "update_title",
      description:
        "Set the session title. Use a concise, descriptive title (max 80 chars) that captures the essence of the session.",
      parameters: {
        type: "object",
        properties: {
          title: {
            type: "string",
            description: "The new session title (max 80 characters)",
          },
        },
        required: ["title"],
      },
    },
  },
  execute: async (args, ctx) => {
    const title = String(args.title).slice(0, 80);
    if (title === ctx.currentTitle) return null;
    const previousTitle = ctx.currentTitle;
    await updateSessionTitle(ctx.sessionId, title);
    return {
      name: "update_title",
      label: "Title",
      detail: title,
      undoData: previousTitle,
    };
  },
  undo: async (undoData, ctx) => {
    await updateSessionTitle(ctx.sessionId, String(undoData));
  },
});

// --- Tool: save_to_notes ---

registerTool({
  affects: ["notes"],
  schema: {
    type: "function",
    function: {
      name: "save_to_notes",
      description:
        "Save content to the session notes. Use 'append' to add alongside existing content, or 'replace' to overwrite (only when notes are empty or you're producing a full rewrite that incorporates existing content). Content should be markdown formatted.",
      parameters: {
        type: "object",
        properties: {
          content: {
            type: "string",
            description: "Markdown content to save to notes",
          },
          mode: {
            type: "string",
            enum: ["replace", "append"],
            description:
              "append: add content below existing notes with a separator. replace: overwrite all notes (use when notes are empty or when your content is a complete rewrite incorporating existing material).",
          },
        },
        required: ["content", "mode"],
      },
    },
  },
  execute: async (args, ctx) => {
    const content = String(args.content);
    const mode = args.mode === "append" ? "append" : "replace";
    const previousContent = ctx.currentNote?.content ?? null;

    let html = markdownToBasicHtml(content);
    if (ctx.segments?.length) {
      html = convertCitationsToSegmentRefs(html, ctx.segments);
    }
    let mergedHtml: string;

    if (
      mode === "append" &&
      previousContent &&
      previousContent !== "<p></p>"
    ) {
      mergedHtml = previousContent + "<hr>" + html;
    } else {
      mergedHtml = html;
    }

    await saveNote(ctx.sessionId, mergedHtml);

    return {
      name: "save_to_notes",
      label: "Notes",
      detail: mode === "append" ? "Appended to notes" : "Notes saved",
      undoData: previousContent,
    };
  },
  undo: async (undoData, ctx) => {
    if (undoData === null) {
      // No previous note — save empty to effectively clear
      await saveNote(ctx.sessionId, "<p></p>");
    } else {
      await saveNote(ctx.sessionId, String(undoData));
    }
  },
});

// --- Tool: pin_session ---

registerTool({
  affects: ["session-meta"],
  schema: {
    type: "function",
    function: {
      name: "pin_session",
      description: "Pin or unpin the session.",
      parameters: {
        type: "object",
        properties: {
          pinned: {
            type: "boolean",
            description: "Whether to pin (true) or unpin (false) the session",
          },
        },
        required: ["pinned"],
      },
    },
  },
  execute: async (args, ctx) => {
    const wantPinned = Boolean(args.pinned);
    const wasPinned = ctx.isPinned;

    if (wantPinned !== wasPinned) {
      await togglePin(ctx.sessionId);
    }

    return {
      name: "pin_session",
      label: "Pinned",
      detail: wantPinned ? "Session pinned" : "Session unpinned",
      undoData: wasPinned,
    };
  },
  undo: async (undoData, ctx) => {
    const wasPinned = Boolean(undoData);
    const session = await getSession(ctx.sessionId);
    const currentlyPinned = session ? session.is_pinned === 1 : false;
    if (wasPinned !== currentlyPinned) {
      await togglePin(ctx.sessionId);
    }
  },
});
