import { useState, useRef, useEffect, useCallback } from "react";
import type { AIContextValue } from "@/lib/ai-context";
import { assembleFolderTreeForActions } from "@/lib/ai-context";
import type { ChatMessage, FileAttachment, ToolExecution } from "@/lib/ai";
import type { ActionDefinition } from "@/lib/ai-actions";
import type {
  ExecutedTool,
  PersistedToolCall,
  ToolCallResult,
  ToolObservation,
} from "@/lib/ai-tools";
import type { ChatCompletionMessageParam } from "openai/resources/chat/completions";
import type { DbChatMessage } from "@/lib/db";
import {
  getChatMessages,
  insertChatMessage,
  updateChatMessageContent,
  deleteChatMessages,
} from "@/lib/db";
import {
  createAIClient,
  getActiveConfig,
  streamChatWithTools,
  DEFAULT_AI_SETTINGS,
} from "@/lib/ai";
import { GENERAL_DIRECTIVE } from "@/lib/ai-prompts";
import { getToolsById, executeTool, undoToolCalls, getToolKind } from "@/lib/ai-tools";
import { useAppStore } from "@/stores/appStore";
import { toast } from "sonner";
import {
  trackChatMessageSent,
  trackChatToolExecuted,
  trackChatToolUndone,
  trackChatCleared,
} from "@/lib/analytics";

/**
 * Convert one persisted row into a ChatMessage for the renderer. Tool rows
 * (`role === "tool"`) aren't user-facing bubbles — they get folded into the
 * surrounding assistant bubble at the renderer layer. Returns `null` for
 * those so the caller can `.filter()` them out.
 */
function dbToChatMessage(row: DbChatMessage): ChatMessage | null {
  if (row.role === "tool") return null;
  let toolExecutions: ToolExecution[] | undefined;
  let content = row.content;
  if (row.tool_calls) {
    try {
      const parsed = JSON.parse(row.tool_calls) as PersistedToolCall[];
      toolExecutions = parsed.map((p) => ({
        name: p.name,
        label: p.label,
        status: p.status,
        detail: p.detail,
      }));
      // Structured calls supersede the legacy `[tool:NAME] …` badge prefix.
      // Strip it so the same message doesn't render the badges twice.
      content = cleanToolBadges(content);
    } catch (e) {
      console.warn(`[chat] failed to parse tool_calls JSON for ${row.id}: ${e}`);
    }
  }
  return {
    id: row.id,
    role: row.role,
    content,
    action: row.action ?? undefined,
    toolExecutions,
  };
}

const UNDO_TIMEOUT_MS = 10_000;

const TOOL_DISPLAY_LABELS: Record<string, string> = {
  update_title: "Updating title",
  save_to_notes: "Saving notes",
  pin_session: "Pinning session",
  tag_session: "Adding tags",
  search_folders: "Searching folders",
  add_session_to_folder: "Classifying session",
  search_sessions: "Searching sessions",
  get_session_context: "Reading sessions",
};

function cleanToolBadges(content: string): string {
  return content.split("\n").filter((l) => !/^\[tool:\w+\]/.test(l)).join("\n").trimStart();
}

export interface UndoState {
  messageId: string;
  executed: ExecutedTool[];
  timer: ReturnType<typeof setTimeout>;
}

export interface UseChatMessagesReturn {
  messages: ChatMessage[];
  isStreaming: boolean;
  undoState: UndoState | null;
  handleSend: (actionDef?: ActionDefinition) => Promise<void>;
  handleUndo: (state: UndoState) => Promise<void>;
  handleClearChat: () => Promise<void>;
}

/** Manages AI chat message state, streaming, tool execution, and DB persistence. */
export function useChatMessages(
  ctx: AIContextValue | null,
  input: string,
  setInput: (value: string) => void,
  attachments: FileAttachment[],
  setIsExpanded: (value: boolean) => void,
): UseChatMessagesReturn {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [undoState, setUndoState] = useState<UndoState | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const messagesRef = useRef(messages);
  messagesRef.current = messages;
  const inputRef = useRef(input);
  inputRef.current = input;

  const aiSettings = useAppStore((s) => s.settings.ai) ?? DEFAULT_AI_SETTINGS;
  const activeConfig = getActiveConfig(aiSettings);

  const contextKey = ctx?.contextKey ?? "";
  const sources = ctx?.sources ?? [];
  const ctxTools = ctx?.tools;
  const buildSystemPrompt = ctx?.buildSystemPrompt;
  const isSessionContext = ctx?.isSessionContext ?? false;
  const sessionId = ctx?.sessionId ?? null;
  const onToolsExecuted = ctx?.onToolsExecuted;

  // Load messages from DB on context change
  useEffect(() => {
    if (!contextKey) return;
    setIsStreaming(false);
    if (undoState) {
      clearTimeout(undoState.timer);
      setUndoState(null);
    }
    getChatMessages(contextKey).then((rows) => {
      setMessages(rows.map(dbToChatMessage).filter((m): m is ChatMessage => m !== null));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contextKey]);

  // Abort streaming on unmount
  useEffect(() => {
    return () => {
      abortRef.current?.abort();
    };
  }, []);

  const handleUndo = useCallback(
    async (state: UndoState) => {
      if (!isSessionContext || !sessionId) return;
      clearTimeout(state.timer);
      try {
        await undoToolCalls(state.executed, { sessionId });
        for (const t of state.executed) {
          trackChatToolUndone({ tool_name: t.name });
        }

        setMessages((prev) =>
          prev.map((m) => {
            if (m.id !== state.messageId) return m;
            return { ...m, content: cleanToolBadges(m.content) };
          }),
        );

        const msg = messagesRef.current.find((m) => m.id === state.messageId);
        if (msg) {
          await updateChatMessageContent(state.messageId, cleanToolBadges(msg.content));
        }

        if (onToolsExecuted) {
          await onToolsExecuted(state.executed.map((t) => t.name));
        }
        toast.success("Reverted");
      } catch (e) {
        toast.error(
          `Failed to undo: ${e instanceof Error ? e.message : String(e)}`,
        );
      }
      setUndoState(null);
    },
    [isSessionContext, sessionId, onToolsExecuted],
  );

  const handleClearChat = useCallback(async () => {
    if (!contextKey) return;
    if (undoState) {
      clearTimeout(undoState.timer);
      setUndoState(null);
    }
    await deleteChatMessages(contextKey);
    trackChatCleared();
    setMessages([]);
    setIsExpanded(false);
  }, [contextKey, undoState, setIsExpanded]);

  const handleSend = useCallback(
    async (actionDef?: ActionDefinition) => {
      if (!buildSystemPrompt || !ctxTools) return;

      const userText = inputRef.current.trim();
      const directive = actionDef?.directive ?? GENERAL_DIRECTIVE;
      const actionId = actionDef?.id ?? "general";

      if (!actionDef && !userText) return;

      const userMessage: ChatMessage = {
        id: crypto.randomUUID(),
        role: "user",
        content: actionDef && !userText ? actionDef.label : userText,
        action: actionId,
      };

      const assistantMessage: ChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: "",
        isStreaming: true,
      };

      setMessages((prev) => [...prev, userMessage, assistantMessage]);
      setInput("");
      setIsStreaming(true);
      setIsExpanded(true);
      trackChatMessageSent({
        context: contextKey.split(":")[0] ?? "unknown",
        has_action: actionDef ? 1 : 0,
        action_id: actionId,
      });

      let flushTimer: ReturnType<typeof setInterval> | null = null;

      try {
        // Persist the placeholder rows inside the try so any DB error (a
        // missing column on a stale dev DB, a locked file, etc.) routes
        // through the catch + finally and clears `isStreaming` instead of
        // surfacing as an unhandled rejection that strands the spinner.
        const sendId = crypto.randomUUID();
        await insertChatMessage({
          id: userMessage.id,
          context_key: contextKey,
          session_id: sessionId,
          role: "user",
          content: userMessage.content,
          action: userMessage.action ?? null,
          created_at: new Date().toISOString(),
          tool_calls: null,
          send_id: sendId,
          sequence: 0,
          tool_call_id: null,
          observation: null,
          status: null,
        });

        await insertChatMessage({
          id: assistantMessage.id,
          context_key: contextKey,
          session_id: sessionId,
          role: "assistant",
          content: "",
          action: null,
          created_at: new Date().toISOString(),
          tool_calls: null,
          send_id: sendId,
          sequence: 1,
          tool_call_id: null,
          observation: null,
          status: null,
        });

        const client = createAIClient(aiSettings);
        const abort = new AbortController();
        abortRef.current = abort;

        let accumulated = "";
        const executedTools: ExecutedTool[] = [];
        const allToolExecs: ToolExecution[] = [];
        const allCallIds: string[] = [];
        const observationsByCallId = new Map<string, ToolObservation>();
        const history = messagesRef.current.filter((m) => m.content);

        const contextParts: Record<string, string> = {};
        for (const source of sources) {
          if (source.enabled) {
            contextParts[source.id] = await source.assembler();
          }
        }

        if (actionDef && isSessionContext) {
          const folderTree = await assembleFolderTreeForActions();
          if (folderTree) {
            contextParts["folder-tree"] = folderTree;
          }
        }

        const systemPrompt = await buildSystemPrompt(
          directive,
          contextParts,
          attachments,
        );

        const chatMessages: ChatCompletionMessageParam[] = [
          { role: "system", content: systemPrompt },
        ];
        for (const msg of history) {
          chatMessages.push({ role: msg.role, content: msg.content });
        }
        const userContent =
          actionDef && !userText ? actionDef.label : userText;
        if (userContent) {
          chatMessages.push({ role: "user", content: userContent });
        }

        const tools = getToolsById(ctxTools.availableToolIds);

        let needsFlush = false;
        const flush = () => {
          if (!needsFlush) return;
          needsFlush = false;
          const current = accumulated;
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantMessage.id
                ? { ...m, content: current }
                : m,
            ),
          );
        };
        flushTimer = setInterval(flush, 50);

        // Multi-turn tool execution: after each LLM turn, if it emits tool_calls,
        // execute them and send results back for a follow-up turn. This lets the
        // LLM use tool results (e.g., folder context from add_session_to_folder)
        // to inform subsequent output (e.g., writing a summary).
        //
        // The cap allows up to MAX_TOOL_ROUNDS tool-calling rounds; after that,
        // one final round runs without tools so the model is forced to
        // synthesize a textual answer over the accumulated observations
        // instead of leaving the user with a row of tool badges and no prose.
        const MAX_TOOL_ROUNDS = 3;
        let turnMessages = chatMessages;

        for (let round = 0; round <= MAX_TOOL_ROUNDS; round++) {
          const allowToolsThisRound = round < MAX_TOOL_ROUNDS;
          const turnTools = allowToolsThisRound ? tools : [];
          let turnText = "";
          let turnToolCalls: ToolCallResult[] = [];

          for await (const event of streamChatWithTools(
            client,
            activeConfig.model,
            turnMessages,
            turnTools,
            abort.signal,
          )) {
            if (event.type === "token") {
              accumulated += event.content;
              turnText += event.content;
              needsFlush = true;
            } else if (event.type === "tool_calls") {
              turnToolCalls = event.calls;
            }
          }

          if (turnToolCalls.length === 0 || !ctxTools.getToolContext) break;
          if (!allowToolsThisRound) break;
          if (abort.signal.aborted) break;

          const pendingExecs: ToolExecution[] = turnToolCalls.map((c) => ({
            name: c.name,
            label: TOOL_DISPLAY_LABELS[c.name] ?? c.name,
            status: "running" as const,
          }));
          allToolExecs.push(...pendingExecs);
          allCallIds.push(...turnToolCalls.map((c) => c.id));

          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantMessage.id
                ? { ...m, content: accumulated, toolExecutions: [...allToolExecs] }
                : m,
            ),
          );

          // Re-fetch between mutating calls inside the same batch — see the
          // refresh after `executeTool` below — so the second mutation can
          // observe the first one's pre-state (folder memberships, tag rows,
          // pin state).
          let toolCtx = await ctxTools.getToolContext();
          const toolResultMessages: ChatCompletionMessageParam[] = [];

          const assistantToolCalls = turnToolCalls.map((call) => ({
            id: call.id,
            type: "function" as const,
            function: { name: call.name, arguments: JSON.stringify(call.arguments) },
          }));

          turnMessages = [
            ...turnMessages,
            {
              role: "assistant" as const,
              content: turnText || null,
              tool_calls: assistantToolCalls,
            },
          ];

          for (let ci = 0; ci < turnToolCalls.length; ci++) {
            const call = turnToolCalls[ci];
            if (abort.signal.aborted) break;
            const execIdx = allToolExecs.length - turnToolCalls.length + ci;
            // streamChatWithTools surfaces malformed JSON arguments by
            // attaching `__parseError`. Skip execution and tell the model
            // so it can retry instead of running a tool with garbage args.
            if (call.arguments.__parseError !== undefined) {
              const errMsg = `Invalid JSON for ${call.name}: ${String(call.arguments.__parseError)}`;
              allToolExecs[execIdx] = {
                ...allToolExecs[execIdx],
                status: "error",
                detail: errMsg,
              };
              toolResultMessages.push({
                role: "tool" as const,
                tool_call_id: call.id,
                content: `Error: ${errMsg}`,
              });
              continue;
            }
            try {
              const result = await executeTool(
                call.name,
                call.arguments,
                toolCtx,
              );
              // After a mutating tool runs, the snapshot the orchestrator
              // captured at the start of this batch (folder ids, tag rows,
              // pin state) is stale. Refresh before the next call so the
              // following tool sees the post-mutation world. Read-only tools
              // can't move state, so skip the round trip for them.
              if (
                getToolKind(call.name) === "mutate" &&
                ctxTools.getToolContext &&
                ci + 1 < turnToolCalls.length
              ) {
                toolCtx = await ctxTools.getToolContext();
              }
              if (result) {
                result.toolCallId = call.id;
                trackChatToolExecuted({ tool_name: call.name });
                allToolExecs[execIdx] = { ...allToolExecs[execIdx], status: "done", detail: result.detail };
                if (result.observation) {
                  observationsByCallId.set(call.id, result.observation);
                }
                // Read-only tools never enter the Undo window. Mutating
                // tools that returned without populating undoData (no-op
                // paths like pin_session-already-pinned, add_session_to_folder-
                // already-in-folder, tag_session-no-real-deltas) also skip
                // the undo state — the LLM still sees the result via
                // toolResultMessages, but the UI doesn't surface a
                // "Session updated" toast for a no-op.
                if (
                  getToolKind(call.name) === "mutate" &&
                  result.undoData !== undefined
                ) {
                  executedTools.push(result);
                }
                toolResultMessages.push({
                  role: "tool" as const,
                  tool_call_id: call.id,
                  content: result.result ?? result.detail,
                });
              } else {
                allToolExecs[execIdx] = { ...allToolExecs[execIdx], status: "done" };
                toolResultMessages.push({
                  role: "tool" as const,
                  tool_call_id: call.id,
                  content: "No action needed.",
                });
              }
            } catch (e) {
              console.error(`Tool ${call.name} failed:`, e);
              allToolExecs[execIdx] = {
                ...allToolExecs[execIdx],
                status: "error",
                detail: e instanceof Error ? e.message : String(e),
              };
              toast.error(
                `Tool failed: ${e instanceof Error ? e.message : String(e)}`,
              );
              toolResultMessages.push({
                role: "tool" as const,
                tool_call_id: call.id,
                content: `Error: ${e instanceof Error ? e.message : String(e)}`,
              });
            }

            setMessages((prev) =>
              prev.map((m) =>
                m.id === assistantMessage.id
                  ? { ...m, toolExecutions: [...allToolExecs] }
                  : m,
              ),
            );
          }

          turnMessages = [...turnMessages, ...toolResultMessages];
        }

        clearInterval(flushTimer);
        flush();

        const finalToolExecs = allToolExecs.length > 0 ? [...allToolExecs] : undefined;
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantMessage.id
              ? { ...m, content: accumulated, isStreaming: false, toolExecutions: finalToolExecs }
              : m,
          ),
        );

        const persistedToolCalls: PersistedToolCall[] | null =
          allToolExecs.length > 0
            ? allToolExecs.map((exec, i) => {
                const observation = observationsByCallId.get(allCallIds[i]);
                const status: PersistedToolCall["status"] =
                  exec.status === "error" ? "error" : "done";
                return {
                  name: exec.name,
                  label: exec.label,
                  status,
                  detail: exec.detail,
                  observation,
                };
              })
            : null;
        await updateChatMessageContent(
          assistantMessage.id,
          accumulated,
          persistedToolCalls ? JSON.stringify(persistedToolCalls) : null,
        );

        if (executedTools.length > 0) {
          const toolNames = executedTools.map((t) => t.label).join(", ");

          if (onToolsExecuted) {
            await onToolsExecuted(executedTools.map((t) => t.name));
          }

          const timer = setTimeout(() => {
            setUndoState(null);
          }, UNDO_TIMEOUT_MS);

          const newUndoState: UndoState = {
            messageId: assistantMessage.id,
            executed: executedTools,
            timer,
          };
          setUndoState(newUndoState);

          toast(`Session updated: ${toolNames}`, {
            action: {
              label: "Undo",
              onClick: () => handleUndo(newUndoState),
            },
            duration: UNDO_TIMEOUT_MS,
          });
        }
      } catch (e) {
        if ((e as Error).name === "AbortError") return;
        const errorText =
          e instanceof Error ? e.message : "An error occurred";
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantMessage.id
              ? { ...m, content: `Error: ${errorText}`, isStreaming: false }
              : m,
          ),
        );
        await updateChatMessageContent(
          assistantMessage.id,
          `Error: ${errorText}`,
        );
      } finally {
        if (flushTimer) clearInterval(flushTimer);
        setIsStreaming(false);
        abortRef.current = null;
      }
    },
    [
      contextKey,
      sessionId,
      isSessionContext,
      sources,
      ctxTools,
      attachments,
      aiSettings,
      activeConfig.model,
      buildSystemPrompt,
      onToolsExecuted,
      handleUndo,
      setInput,
      setIsExpanded,
    ],
  );

  return {
    messages,
    isStreaming,
    undoState,
    handleSend,
    handleUndo,
    handleClearChat,
  };
}
