import { createContext, useContext, useState, useEffect, useMemo, useCallback } from "react";
import type { ReactNode } from "react";
import type { AIContextValue, ContextSource, AIContextTools, SystemPromptBuilder } from "@/lib/ai-context";
import type { ActionDefinition } from "@/lib/ai-actions";
import type { DbSegment } from "@/lib/db";

const AIContext = createContext<AIContextValue | null>(null);

export function useAIContext(): AIContextValue | null {
  return useContext(AIContext);
}

interface AIContextProviderProps {
  contextKey: string;
  sources: ContextSource[];
  tools: AIContextTools;
  actions: ActionDefinition[];
  segments: DbSegment[];
  buildSystemPrompt: SystemPromptBuilder;
  isSessionContext: boolean;
  sessionId: string | null;
  onToolsExecuted: (toolNames: string[]) => Promise<void>;
  placeholder?: string;
  children: ReactNode;
}

export function AIContextProvider({
  contextKey,
  sources,
  tools,
  actions,
  segments,
  buildSystemPrompt,
  isSessionContext,
  sessionId,
  onToolsExecuted,
  placeholder,
  children,
}: AIContextProviderProps) {
  const [disabledSources, setDisabledSources] = useState<Set<string>>(new Set());

  // Reset disabled sources when context changes
  useEffect(() => {
    setDisabledSources(new Set());
  }, [contextKey]);

  const toggleSource = useCallback((sourceId: string) => {
    setDisabledSources((prev) => {
      const next = new Set(prev);
      if (next.has(sourceId)) {
        next.delete(sourceId);
      } else {
        next.add(sourceId);
      }
      return next;
    });
  }, []);

  // Merge toggle state into sources
  const mergedSources = useMemo(
    () =>
      sources.map((s) => ({
        ...s,
        enabled: s.toggleable ? !disabledSources.has(s.id) : s.enabled,
      })),
    [sources, disabledSources],
  );

  const value = useMemo<AIContextValue>(
    () => ({
      contextKey,
      sources: mergedSources,
      toggleSource,
      tools,
      actions,
      segments,
      buildSystemPrompt,
      isSessionContext,
      sessionId,
      onToolsExecuted,
      placeholder: placeholder ?? (isSessionContext ? "Ask about your notes..." : "Ask a question..."),
    }),
    [
      contextKey,
      mergedSources,
      toggleSource,
      tools,
      actions,
      segments,
      buildSystemPrompt,
      isSessionContext,
      sessionId,
      onToolsExecuted,
      placeholder,
    ],
  );

  return <AIContext.Provider value={value}>{children}</AIContext.Provider>;
}
