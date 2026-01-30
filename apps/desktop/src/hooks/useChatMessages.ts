import { useState, useRef, useEffect, useCallback } from "react";
import type { AIContextValue } from "@/lib/ai-context";
import type { ChatMessage, FileAttachment } from "@/lib/ai";
import type { ActionDefinition } from "@/lib/ai-actions";
import type { ExecutedTool } from "@/lib/ai-tools";
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
import { getToolsById, executeTool, undoToolCalls } from "@/lib/ai-tools";
import { useAppStore } from "@/stores/appStore";
import { toast } from "sonner";
import {
  trackChatMessageSent,
  trackChatToolExecuted,
  trackChatToolUndone,
  trackChatCleared,
} from "@/lib/analytics";

function dbToChatMessage(row: DbChatMessage): ChatMessage {
  return {
    id: row.id,
    role: row.role,
    content: row.content,
    action: row.action ?? undefined,
  };
}

const UNDO_TIMEOUT_MS = 10_000;

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
      setMessages(rows.map(dbToChatMessage));
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

      await insertChatMessage({
        id: userMessage.id,
        context_key: contextKey,
        session_id: sessionId,
        role: "user",
        content: userMessage.content,
        action: userMessage.action ?? null,
        created_at: new Date().toISOString(),
      });

      await insertChatMessage({
        id: assistantMessage.id,
        context_key: contextKey,
        session_id: sessionId,
        role: "assistant",
        content: "",
        action: null,
        created_at: new Date().toISOString(),
      });

      let flushTimer: ReturnType<typeof setInterval> | null = null;

      try {
        const client = createAIClient(aiSettings);
        const abort = new AbortController();
        abortRef.current = abort;

        let accumulated = "";
        const executedTools: ExecutedTool[] = [];
        const history = messagesRef.current.filter((m) => m.content);

        const contextParts: Record<string, string> = {};
        for (const source of sources) {
          if (source.enabled) {
            contextParts[source.id] = await source.assembler();
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

        for await (const event of streamChatWithTools(
          client,
          activeConfig.model,
          chatMessages,
          tools,
          abort.signal,
        )) {
          if (event.type === "token") {
            accumulated += event.content;
            needsFlush = true;
          } else if (event.type === "tool_calls" && ctxTools.getToolContext) {
            const toolCtx = await ctxTools.getToolContext();

            for (const call of event.calls) {
              try {
                const result = await executeTool(
                  call.name,
                  call.arguments,
                  toolCtx,
                );
                if (result) {
                  executedTools.push(result);
                  trackChatToolExecuted({ tool_name: call.name });
                }
              } catch (e) {
                console.error(`Tool ${call.name} failed:`, e);
                toast.error(
                  `Tool failed: ${e instanceof Error ? e.message : String(e)}`,
                );
              }
            }
          }
        }
        clearInterval(flushTimer);
        flush();

        if (executedTools.length > 0) {
          const badgeLines = executedTools
            .map((t) => `[tool:${t.name}] ${t.detail}`)
            .join("\n");
          accumulated = badgeLines + "\n" + accumulated;
        }

        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantMessage.id
              ? { ...m, content: accumulated, isStreaming: false }
              : m,
          ),
        );

        await updateChatMessageContent(assistantMessage.id, accumulated);

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
