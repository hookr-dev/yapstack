import type { ChatCompletionTool } from "openai/resources/chat/completions";
import {
  updateSessionTitle,
  saveNote,
  togglePin,
  getSession,
  getTagByName,
  createTag,
  addSessionTag,
  removeSessionTag,
  getSessionTagRows,
  addSessionToFolder as dbAddSessionToFolder,
  removeSessionFromFolder as dbRemoveSessionFromFolder,
  listFolders,
} from "./db";
import type { DbNote, DbSegment } from "./db";
import { markdownToBasicHtml } from "./ai";
import { formatTime } from "./utils";
import { findBranchConflicts, getFolderPath, buildFolderTree } from "./folder-tree";
import { assembleFolderTreeContext } from "./ai";

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
  toolCallId?: string;
  result?: string;
  undoData?: unknown;
  /**
   * True when the tool changed session state (title, notes, pin, tags,
   * folder membership). Read-only or no-op results set this to false so
   * the chat UI doesn't render an Undo toast / "Session updated" message
   * / refresh callback for something that didn't mutate.
   */
  mutated?: boolean;
}

export interface ToolContext {
  sessionId: string;
  currentTitle: string;
  currentNote: DbNote | null;
  isPinned: boolean;
  segments?: DbSegment[];
  tags?: string[];
  folderNames?: string[];
  folderIds?: string[];
}

// --- Modular tool definition ---

export type ToolEffect = "session-meta" | "notes" | "organization";

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
      result: `Title updated from "${previousTitle}" to "${title}".`,
      undoData: previousTitle,
      mutated: true,
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

    const wordCount = content.split(/\s+/).filter(Boolean).length;
    return {
      name: "save_to_notes",
      label: "Notes",
      detail: mode === "append" ? "Appended to notes" : "Notes saved",
      result: `Notes ${mode === "append" ? "appended" : "saved"} successfully (${wordCount} words).`,
      undoData: previousContent,
      mutated: true,
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
    const changed = wantPinned !== wasPinned;

    if (changed) {
      await togglePin(ctx.sessionId);
    }

    return {
      name: "pin_session",
      label: "Pinned",
      detail: wantPinned ? "Session pinned" : "Session unpinned",
      result: changed
        ? wantPinned
          ? "Session pinned successfully."
          : "Session unpinned successfully."
        : `Session was already ${wantPinned ? "pinned" : "unpinned"}.`,
      undoData: wasPinned,
      mutated: changed,
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

// --- Tool: tag_session ---

registerTool({
  affects: ["organization"],
  schema: {
    type: "function",
    function: {
      name: "tag_session",
      description:
        "Add or remove tags from the current session. Creates new tags automatically if they don't exist yet.",
      parameters: {
        type: "object",
        properties: {
          add: {
            type: "array",
            items: { type: "string" },
            description: "Tag names to add to the session",
          },
          remove: {
            type: "array",
            items: { type: "string" },
            description: "Tag names to remove from the session",
          },
        },
        required: ["add"],
      },
    },
  },
  execute: async (args, ctx) => {
    const toAdd = (args.add as string[]) ?? [];
    const toRemove = (args.remove as string[]) ?? [];

    // Snapshot existing rows so undo only inverts real deltas. Without this,
    // INSERT-OR-IGNORE adds and unconditional removes would record phantom
    // deltas that, on undo, drop pre-existing manual tags or re-add tags
    // that were never on the session.
    const beforeRows = await getSessionTagRows(ctx.sessionId);
    const beforeSourceById = new Map(
      beforeRows.map((r) => [r.tag_id, r.source]),
    );

    const addedTagIds: string[] = [];
    const removedTags: { tagId: string; source: "manual" | "auto" | "ai" }[] = [];
    const addedNames: string[] = [];
    const removedNames: string[] = [];

    for (const name of toAdd) {
      const trimmed = name.trim();
      if (!trimmed) continue;
      let tag = await getTagByName(trimmed);
      if (!tag) {
        const id = crypto.randomUUID();
        await createTag(id, trimmed);
        tag = { id, name: trimmed, color: null, created_at: new Date().toISOString() };
      }
      if (beforeSourceById.has(tag.id)) continue; // already on the session
      await addSessionTag(ctx.sessionId, tag.id, "ai");
      addedTagIds.push(tag.id);
      addedNames.push(trimmed);
    }

    for (const name of toRemove) {
      const trimmed = name.trim();
      if (!trimmed) continue;
      const tag = await getTagByName(trimmed);
      if (!tag) continue;
      const previousSource = beforeSourceById.get(tag.id);
      if (previousSource === undefined) continue; // wasn't on the session
      await removeSessionTag(ctx.sessionId, tag.id);
      removedTags.push({ tagId: tag.id, source: previousSource });
      removedNames.push(trimmed);
    }

    const parts: string[] = [];
    if (addedNames.length > 0) parts.push(`Tags added: ${addedNames.join(", ")}`);
    if (removedNames.length > 0)
      parts.push(`Tags removed: ${removedNames.join(", ")}`);
    const detail = parts.length > 0 ? parts.join(". ") : "No tag changes";
    const result =
      parts.length > 0 ? parts.join(". ") + "." : "Tags already up to date.";

    const mutated = addedTagIds.length > 0 || removedTags.length > 0;
    return {
      name: "tag_session",
      label: "Tags",
      detail,
      result,
      undoData: mutated ? { addedTagIds, removedTags } : undefined,
      mutated,
    };
  },
  undo: async (undoData, ctx) => {
    const data = undoData as {
      addedTagIds: string[];
      removedTags: { tagId: string; source: "manual" | "auto" | "ai" }[];
    };
    for (const tagId of data.addedTagIds) {
      await removeSessionTag(ctx.sessionId, tagId);
    }
    for (const { tagId, source } of data.removedTags) {
      await addSessionTag(ctx.sessionId, tagId, source);
    }
  },
});

// --- Folder context helpers ---

import type { DbFolder } from "./db";

function formatFolderContextChain(folders: DbFolder[], folderId: string): string {
  const path = getFolderPath(folders, folderId);
  if (path.length === 0) return "";
  return path
    .map((f) => {
      const desc = f.description ? ` — ${f.description}` : "";
      return `${f.name}${desc}`;
    })
    .join(" > ");
}

// --- Tool: add_to_folder ---

registerTool({
  affects: ["organization"],
  schema: {
    type: "function",
    function: {
      name: "add_to_folder",
      description:
        "Add the session to a folder by name. If the session is already in a conflicting ancestor or descendant folder, it will be moved. Use this to classify sessions into the correct organizational folder. Returns the folder's hierarchical context.",
      parameters: {
        type: "object",
        properties: {
          folder_name: {
            type: "string",
            description: "The exact name of the folder to add the session to",
          },
        },
        required: ["folder_name"],
      },
    },
  },
  execute: async (args, ctx) => {
    const folderName = String(args.folder_name).trim();
    const folders = await listFolders();
    const matches = folders.filter(
      (f) => f.name.toLowerCase() === folderName.toLowerCase(),
    );
    if (matches.length === 0) {
      return {
        name: "add_to_folder",
        label: "Folder",
        detail: `Folder "${folderName}" not found`,
        result: `Error: No folder named "${folderName}" exists.`,
        mutated: false,
      };
    }
    if (matches.length > 1) {
      // Multiple folders share this name under different parents. Refuse to
      // guess — surface the candidate paths so the LLM (or a follow-up call)
      // can pick the intended one. Today the schema only takes a name; the
      // model can resolve by re-calling get_folder_context for the tree, or
      // by asking the user.
      const paths = matches
        .map((m) => formatFolderContextChain(folders, m.id))
        .filter((p) => p.length > 0);
      return {
        name: "add_to_folder",
        label: "Folder",
        detail: `Ambiguous: ${matches.length} folders named "${folderName}"`,
        result: `Error: ${matches.length} folders named "${folderName}" exist. Resolve by parent path before retrying. Candidates:\n${paths.map((p) => `- ${p}`).join("\n")}`,
        mutated: false,
      };
    }

    const target = matches[0];
    const currentFolderIds = ctx.folderIds ?? [];
    if (currentFolderIds.includes(target.id)) {
      const contextChain = formatFolderContextChain(folders, target.id);
      return {
        name: "add_to_folder",
        label: "Folder",
        detail: `Already in "${target.name}"`,
        result: `Session is already in this folder. Context: ${contextChain}`,
        mutated: false,
      };
    }

    const conflicts = findBranchConflicts(folders, currentFolderIds, target.id);
    for (const cId of conflicts) {
      await dbRemoveSessionFromFolder(ctx.sessionId, cId);
    }
    await dbAddSessionToFolder(ctx.sessionId, target.id);

    const contextChain = formatFolderContextChain(folders, target.id);

    return {
      name: "add_to_folder",
      label: "Folder",
      detail: `Added to "${target.name}"`,
      result: `Session added to "${target.name}". Folder context: ${contextChain}. Use this context to inform your summary.`,
      undoData: { addedFolderId: target.id, removedConflicts: conflicts },
      mutated: true,
    };
  },
  undo: async (undoData, ctx) => {
    const data = undoData as { addedFolderId: string; removedConflicts: string[] };
    await dbRemoveSessionFromFolder(ctx.sessionId, data.addedFolderId);
    for (const folderId of data.removedConflicts) {
      await dbAddSessionToFolder(ctx.sessionId, folderId);
    }
  },
});

// --- Tool: get_folder_context ---

registerTool({
  affects: [],
  schema: {
    type: "function",
    function: {
      name: "get_folder_context",
      description:
        "Get the full folder tree with descriptions, or the context chain for a specific folder. Call this to understand the organizational structure before classifying a session, or to retrieve a folder's description for informed summarization.",
      parameters: {
        type: "object",
        properties: {
          folder_name: {
            type: "string",
            description: "Optional: name of a specific folder to get context for. If omitted, returns the full folder tree.",
          },
        },
      },
    },
  },
  execute: async (args) => {
    const folders = await listFolders();
    if (folders.length === 0) {
      return {
        name: "get_folder_context",
        label: "Folders",
        detail: "No folders exist",
        result: "No folders have been created yet.",
        mutated: false,
      };
    }

    const folderName = args.folder_name ? String(args.folder_name).trim() : null;

    if (folderName) {
      const matches = folders.filter(
        (f) => f.name.toLowerCase() === folderName.toLowerCase(),
      );
      if (matches.length === 0) {
        return {
          name: "get_folder_context",
          label: "Folders",
          detail: `Folder "${folderName}" not found`,
          result: `No folder named "${folderName}". Available folders: ${folders.map((f) => f.name).join(", ")}`,
          mutated: false,
        };
      }
      if (matches.length > 1) {
        const paths = matches
          .map((m) => formatFolderContextChain(folders, m.id))
          .filter((p) => p.length > 0);
        return {
          name: "get_folder_context",
          label: "Folders",
          detail: `Ambiguous: ${matches.length} folders named "${folderName}"`,
          result: `${matches.length} folders named "${folderName}" exist. Candidates:\n${paths.map((p) => `- ${p}`).join("\n")}`,
          mutated: false,
        };
      }
      const target = matches[0];
      const contextChain = formatFolderContextChain(folders, target.id);
      return {
        name: "get_folder_context",
        label: "Folders",
        detail: `Context for "${target.name}"`,
        result: `Folder context chain: ${contextChain}`,
        mutated: false,
      };
    }

    const tree = buildFolderTree(folders);
    const treeText = assembleFolderTreeContext(tree);
    return {
      name: "get_folder_context",
      label: "Folders",
      detail: `${folders.length} folders`,
      result: `Folder tree:\n${treeText}`,
      mutated: false,
    };
  },
  undo: async () => {},
});
