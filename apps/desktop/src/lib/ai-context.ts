import type { LucideIcon } from "lucide-react";
import { FileText, StickyNote, Layers, Mic } from "lucide-react";
import type { DbSegment, DbSession, DbFolder, DbDictationHistory } from "./db";
import {
  getSessionSegments,
  getNote,
  getSession,
  getSessionsByIds,
  listDictationHistory,
  getSessionTagIds,
  listAllSessionFolders,
  listTags,
  listFolders,
} from "./db";
import {
  assembleTranscriptContext,
  assembleNoteContext,
  assembleSessionCandidates,
  assembleDictationContext,
  assembleFolderTreeContext,
  chatContextKey,
  transcriptHasSpeakers,
} from "./ai";
import type { ChatContext, FileAttachment } from "./ai";
import { useAppStore } from "@/stores/appStore";
import {
  getSystemPromptWithToolContext,
  getMultiSessionSystemPrompt,
  getDictationSystemPrompt,
} from "./ai-prompts";
import type { FolderContextLayer } from "./ai-prompts";
import type { ToolContext } from "./ai-tools";
import type { ActionDefinition } from "./ai-actions";
import { getFolderPath, buildFolderTree } from "./folder-tree";

export type ListChatContext = Exclude<ChatContext, { type: "session" }>;

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
        const speakerNames =
          useAppStore.getState().settings.speakerNames[sessionId];
        return assembleTranscriptContext(segments, speakerNames);
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
    availableToolIds: [
      "update_title",
      "save_to_notes",
      "pin_session",
      "tag_session",
      "search_folders",
      "add_session_to_folder",
      "search_sessions",
      "get_session_context",
    ],
    contextType: "session",
    getToolContext: async (): Promise<ToolContext> => {
      const [session, note, segments, tagIds, allSessionFolders, allTags] = await Promise.all([
        getSession(sessionId),
        getNote(sessionId),
        getSessionSegments(sessionId),
        getSessionTagIds(sessionId),
        listAllSessionFolders(),
        listTags(),
      ]);
      const folderIds = allSessionFolders
        .filter((sf) => sf.session_id === sessionId)
        .map((sf) => sf.folder_id);
      const tagNames = tagIds
        .map((tid) => allTags.find((t) => t.id === tid)?.name)
        .filter((n): n is string => !!n);
      return {
        sessionId,
        currentTitle: session?.title ?? "Untitled",
        currentNote: note,
        isPinned: session?.is_pinned === 1,
        segments,
        tags: tagNames,
        folderIds,
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
    const folderTreeCtx = contextParts["folder-tree"] ?? undefined;
    const session = await getSession(sessionId);
    const note = await getNote(sessionId);
    const sessionMeta = {
      title: session?.title ?? "Untitled",
      isPinned: session?.is_pinned === 1,
      hasNotes: !!(note && note.content && note.content !== "<p></p>"),
    };
    // Detect diarization at prompt-build time so the SPEAKER_INSTRUCTION
    // only appears for sessions that actually have speaker labels.
    const segments = await getSessionSegments(sessionId);
    const hasSpeakers = transcriptHasSpeakers(segments);
    return getSystemPromptWithToolContext(
      directive,
      transcript,
      notes,
      attachments,
      sessionMeta,
      folderTreeCtx,
      { hasSpeakers },
    );
  };
}

export async function assembleFolderTreeForActions(): Promise<string> {
  const folders = await listFolders();
  if (folders.length === 0) return "";
  const tree = buildFolderTree(folders);
  return assembleFolderTreeContext(tree);
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
        // Compact candidate list only — title, date, folder path. The LLM
        // pulls richer context (notes, transcript) on demand via the
        // `get_session_context` tool. This stops the prompt from ballooning
        // when the list filter covers many sessions.
        const [sessions, folders, sessionFolders] = await Promise.all([
          getSessionsByIds(sessionIds),
          listFolders(),
          listAllSessionFolders(),
        ]);
        const candidates = sessions.map((s) => {
          const fid = sessionFolders.find((sf) => sf.session_id === s.id)
            ?.folder_id;
          const folderPath = fid
            ? getFolderPath(folders, fid)
                .map((f) => f.name)
                .join(" > ")
            : null;
          return {
            id: s.id,
            title: s.title,
            date: s.created_at,
            folderPath,
          };
        });
        return assembleSessionCandidates(candidates);
      },
    },
  ];
}

export function createMultiSessionTools(): AIContextTools {
  return {
    // Multi-session Chats expose retrieval-only tools so the LLM can search
    // and expand sessions on demand instead of having all candidates dumped
    // into the prompt. No mutating tools here — those need a single
    // sessionId and would need a different ToolContext.
    availableToolIds: ["search_sessions", "get_session_context", "search_folders"],
    contextType: "multi-session",
    // Retrieval tools don't read from session-scoped ToolContext; they hit
    // the DB directly. Provide an empty context so the orchestrator runs
    // the multi-turn loop. The session-meta fields go unused by the
    // retrieval tools.
    getToolContext: async () => ({
      sessionId: "",
      currentTitle: "",
      currentNote: null,
      isPinned: false,
    }),
  };
}

export function createMultiSessionSystemPromptBuilder(
  folderContext?: FolderContextLayer[],
): SystemPromptBuilder {
  return async (_directive, contextParts, attachments) => {
    const sessionsContext = contextParts["sessions"] ?? "";
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
