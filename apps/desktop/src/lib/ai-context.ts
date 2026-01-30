import type { LucideIcon } from "lucide-react";
import { FileText, StickyNote, Layers, Mic } from "lucide-react";
import type { DbSegment, DbSession, DbFolder, DbDictationHistory } from "./db";
import {
  getSessionSegments,
  getNote,
  getSession,
  getNotesForSessions,
  listDictationHistory,
} from "./db";
import {
  assembleTranscriptContext,
  assembleNoteContext,
  assembleMultiSessionContext,
  assembleDictationContext,
  chatContextKey,
} from "./ai";
import type { ChatContext, FileAttachment } from "./ai";

export type ListChatContext = Exclude<ChatContext, { type: "session" }>;

import {
  getSystemPromptWithToolContext,
  getMultiSessionSystemPrompt,
  getDictationSystemPrompt,
} from "./ai-prompts";
import type { FolderContextLayer } from "./ai-prompts";
import type { ToolContext } from "./ai-tools";
import type { ActionDefinition } from "./ai-actions";
import { getFolderPath } from "./folder-tree";

// --- Core types ---

export interface ContextSource {
  id: string;
  type: "transcript" | "notes" | "sessions" | "dictation";
  label: string;
  icon: LucideIcon;
  enabled: boolean;
  toggleable: boolean;
  summary?: string;
  assembler: () => Promise<string>;
}

export interface AIContextTools {
  availableToolIds: string[];
  getToolContext: (() => Promise<ToolContext>) | null;
  contextType?: "session" | "multi-session";
}

export type SystemPromptBuilder = (
  directive: string,
  contextParts: Record<string, string>,
  attachments: FileAttachment[],
) => Promise<string>;

export interface AIContextValue {
  contextKey: string;
  sources: ContextSource[];
  toggleSource: (sourceId: string) => void;
  tools: AIContextTools;
  actions: ActionDefinition[];
  segments: DbSegment[];
  buildSystemPrompt: SystemPromptBuilder;
  isSessionContext: boolean;
  sessionId: string | null;
  onToolsExecuted: (toolNames: string[]) => Promise<void>;
  placeholder: string;
}

// --- Factory functions ---

export function createSessionSources(
  sessionId: string,
  segmentCount: number,
  sessionType: string,
): ContextSource[] {
  const sources: ContextSource[] = [];

  if (sessionType !== "manual") {
    sources.push({
      id: "transcript",
      type: "transcript",
      label: "Transcript",
      icon: FileText,
      enabled: true,
      toggleable: true,
      summary: segmentCount > 0 ? `${segmentCount} segments` : undefined,
      assembler: async () => {
        const segments = await getSessionSegments(sessionId);
        return assembleTranscriptContext(segments);
      },
    });
  }

  sources.push({
    id: "notes",
    type: "notes",
    label: "Note",
    icon: StickyNote,
    enabled: true,
    toggleable: true,
    assembler: async () => {
      const note = await getNote(sessionId);
      if (!note || !note.content || note.content === "<p></p>") return "";
      return assembleNoteContext(note.content);
    },
  });

  return sources;
}

export function createSessionTools(sessionId: string): AIContextTools {
  return {
    availableToolIds: ["update_title", "save_to_notes", "pin_session"],
    contextType: "session",
    getToolContext: async (): Promise<ToolContext> => {
      const session = await getSession(sessionId);
      const note = await getNote(sessionId);
      const segments = await getSessionSegments(sessionId);
      return {
        sessionId,
        currentTitle: session?.title ?? "Untitled",
        currentNote: note,
        isPinned: session?.is_pinned === 1,
        segments,
      };
    },
  };
}

export function createSessionSystemPromptBuilder(
  sessionId: string,
): SystemPromptBuilder {
  return async (directive, contextParts, attachments) => {
    const transcript = contextParts["transcript"] ?? "";
    const notes = contextParts["notes"] ?? "";
    const session = await getSession(sessionId);
    const note = await getNote(sessionId);
    const sessionMeta = {
      title: session?.title ?? "Untitled",
      isPinned: session?.is_pinned === 1,
      hasNotes: !!(note && note.content && note.content !== "<p></p>"),
    };
    return getSystemPromptWithToolContext(
      directive,
      transcript,
      notes,
      attachments,
      sessionMeta,
    );
  };
}

export function createMultiSessionSources(
  sessionIds: string[],
  count: number,
): ContextSource[] {
  return [
    {
      id: "sessions",
      type: "sessions",
      label: `${count} Sessions`,
      icon: Layers,
      enabled: true,
      toggleable: false,
      summary: `${count} sessions`,
      assembler: async () => {
        // Session list without notes — notes handled by separate source
        const sessionNotes = await getNotesForSessions(sessionIds);
        return assembleMultiSessionContext(sessionNotes, false);
      },
    },
    {
      id: "session-notes",
      type: "notes",
      label: "Notes",
      icon: StickyNote,
      enabled: true,
      toggleable: true,
      assembler: async () => {
        // Full context with notes included
        const sessionNotes = await getNotesForSessions(sessionIds);
        return assembleMultiSessionContext(sessionNotes, true);
      },
    },
  ];
}

export function createMultiSessionTools(): AIContextTools {
  return {
    availableToolIds: [],
    contextType: "multi-session",
    getToolContext: null,
  };
}

export function createMultiSessionSystemPromptBuilder(
  folderContext?: FolderContextLayer[],
): SystemPromptBuilder {
  return async (_directive, contextParts, attachments) => {
    // Use notes-inclusive context if notes source is enabled, otherwise sessions-only
    const sessionsContext = contextParts["session-notes"] ?? contextParts["sessions"] ?? "";
    return getMultiSessionSystemPrompt(sessionsContext, attachments, folderContext);
  };
}

export function createDictationSources(count: number): ContextSource[] {
  return [
    {
      id: "dictation",
      type: "dictation",
      label: `${count} Dictation${count !== 1 ? "s" : ""}`,
      icon: Mic,
      enabled: true,
      toggleable: false,
      summary: "dictation history",
      assembler: async () => {
        const entries = await listDictationHistory();
        return assembleDictationContext(entries);
      },
    },
  ];
}

export function createDictationSystemPromptBuilder(): SystemPromptBuilder {
  return async (_directive, contextParts, attachments) => {
    const dictationContext = contextParts["dictation"] ?? "";
    return getDictationSystemPrompt(dictationContext, attachments);
  };
}

// --- List context resolution ---

export interface ListContextConfig {
  contextKey: string;
  sources: ContextSource[];
  tools: AIContextTools;
  buildSystemPrompt: SystemPromptBuilder;
  placeholder: string;
}

export function resolveListContext(
  chatContext: ListChatContext,
  deps: {
    sessions: DbSession[];
    sessionFolderMap: Record<string, string[]>;
    folders: DbFolder[];
    dictationHistory: DbDictationHistory[];
  },
): ListContextConfig {
  const key = chatContextKey(chatContext);

  switch (chatContext.type) {
    case "dictation": {
      const count = deps.dictationHistory.length;
      return {
        contextKey: key,
        sources: createDictationSources(count),
        tools: createMultiSessionTools(),
        buildSystemPrompt: createDictationSystemPromptBuilder(),
        placeholder: `Ask about ${count} dictation${count !== 1 ? "s" : ""}...`,
      };
    }
    case "folder": {
      const ids = deps.sessions
        .filter((s) => (deps.sessionFolderMap[s.id] ?? []).includes(chatContext.folderId))
        .map((s) => s.id);
      const folderPath = getFolderPath(deps.folders, chatContext.folderId);
      const layers = folderPath
        .filter((f): f is DbFolder & { description: string } => !!f.description)
        .map((f) => ({ name: f.name, description: f.description }));
      return {
        contextKey: key,
        sources: createMultiSessionSources(ids, ids.length),
        tools: createMultiSessionTools(),
        buildSystemPrompt: createMultiSessionSystemPromptBuilder(layers.length > 0 ? layers : undefined),
        placeholder: `Ask about ${ids.length} session${ids.length !== 1 ? "s" : ""}...`,
      };
    }
    case "pinned": {
      const ids = deps.sessions.filter((s) => s.is_pinned === 1).map((s) => s.id);
      return {
        contextKey: key,
        sources: createMultiSessionSources(ids, ids.length),
        tools: createMultiSessionTools(),
        buildSystemPrompt: createMultiSessionSystemPromptBuilder(),
        placeholder: `Ask about ${ids.length} pinned session${ids.length !== 1 ? "s" : ""}...`,
      };
    }
    case "all": {
      const ids = deps.sessions.map((s) => s.id);
      return {
        contextKey: key,
        sources: createMultiSessionSources(ids, ids.length),
        tools: createMultiSessionTools(),
        buildSystemPrompt: createMultiSessionSystemPromptBuilder(),
        placeholder: `Ask about ${ids.length} session${ids.length !== 1 ? "s" : ""}...`,
      };
    }
  }
}
