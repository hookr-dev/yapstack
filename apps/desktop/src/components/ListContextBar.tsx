import { useMemo } from "react";
import { useAppStore } from "@/stores/appStore";
import { AIContextProvider } from "@/components/AIContextProvider";
import { FloatingChatBar } from "@/components/FloatingChatBar";
import { resolveListContext } from "@/lib/ai-context";
import type { ListChatContext } from "@/lib/ai-context";
import type { ActionDefinition } from "@/lib/ai-actions";
import type { DbSegment } from "@/lib/db";

const EMPTY_ACTIONS: ActionDefinition[] = [];
const EMPTY_SEGMENTS: DbSegment[] = [];

const noopToolsExecuted = async () => {};

export function ListContextBar({ chatContext }: { chatContext: ListChatContext }) {
  const sessions = useAppStore((s) => s.sessions);
  const sessionFolderMap = useAppStore((s) => s.sessionFolderMap);
  const folders = useAppStore((s) => s.folders);
  const dictationHistory = useAppStore((s) => s.dictationHistory);

  const config = useMemo(
    () => resolveListContext(chatContext, { sessions, sessionFolderMap, folders, dictationHistory }),
    [chatContext, sessions, sessionFolderMap, folders, dictationHistory],
  );

  return (
    <AIContextProvider
      contextKey={config.contextKey}
      sources={config.sources}
      tools={config.tools}
      actions={EMPTY_ACTIONS}
      segments={EMPTY_SEGMENTS}
      buildSystemPrompt={config.buildSystemPrompt}
      isSessionContext={false}
      sessionId={null}
      onToolsExecuted={noopToolsExecuted}
      placeholder={config.placeholder}
    >
      <FloatingChatBar />
    </AIContextProvider>
  );
}
