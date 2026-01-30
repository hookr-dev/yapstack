import { useEffect, useCallback, useMemo, useState } from "react";
import { DndContext, DragOverlay, DragEndEvent, DragStartEvent, PointerSensor, useSensor, useSensors } from "@dnd-kit/core";
import { Mic, PenLine, Pin, Folder } from "lucide-react";
import { isDescendantOf } from "@/lib/folder-tree";
import { useAutoSetup } from "@/hooks/useAutoSetup";
import { useCaptureEvents } from "@/hooks/useCaptureEvents";
import { useDownloadProgress } from "@/hooks/useDownloadProgress";
import { useLiveTranscriptionEvents } from "@/hooks/useLiveTranscriptionEvents";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";
import { useTrayEvents } from "@/hooks/useTrayEvents";
import { useAppStore } from "@/stores/appStore";
import { TitleBar } from "@/components/TitleBar";
import { SetupBanner } from "@/components/SetupBanner";
import { SettingsPanel } from "@/components/SettingsPanel";
import { AppSidebar } from "@/components/AppSidebar";
import { NoteDetailView } from "@/components/NoteDetailView";
import { NoteCardList } from "@/components/NoteCardList";
import { ListContextBar } from "@/components/ListContextBar";
import { OnboardingModal } from "@/components/OnboardingModal";
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable";
import {
  TooltipProvider,
} from "@/components/ui/tooltip";
import { toast } from "sonner";
import type { ChatContext } from "@/lib/ai";

export function AppLayout() {
  useAutoSetup();
  useCaptureEvents();
  useDownloadProgress();
  useLiveTranscriptionEvents();
  useKeyboardShortcuts();
  useTrayEvents();

  const currentView = useAppStore((s) => s.currentView);
  const selectedSessionId = useAppStore((s) => s.selectedSessionId);
  const listFilter = useAppStore((s) => s.listFilter);
  const loadSessions = useAppStore((s) => s.loadSessions);
  const loadFolders = useAppStore((s) => s.loadFolders);
  const loadSessionFolders = useAppStore((s) => s.loadSessionFolders);
  const addSessionToFolder = useAppStore((s) => s.addSessionToFolder);
  const moveFolder = useAppStore((s) => s.moveFolder);
  const reorderFolders = useAppStore((s) => s.reorderFolders);
  const folders = useAppStore((s) => s.folders);
  const sidebarCollapsed = useAppStore((s) => s.settings.sidebarCollapsed);
  const dictationEnabled = useAppStore((s) => s.settings.dictation.enabled);
  const loadDictationHistory = useAppStore((s) => s.loadDictationHistory);

  const pointerSensor = useSensor(PointerSensor, {
    activationConstraint: { distance: 5 },
  });
  const sensors = useSensors(pointerSensor);

  const [activeSession, setActiveSession] = useState<{
    id: string;
    title: string;
    sessionType: string;
    isPinned: boolean;
    totalSegments: number;
    createdAt: string;
  } | null>(null);

  const [activeFolder, setActiveFolder] = useState<{
    id: string;
    name: string;
  } | null>(null);

  useEffect(() => {
    loadSessions();
    loadFolders();
    loadSessionFolders();
  }, [loadSessions, loadFolders, loadSessionFolders]);

  // Load dictation history unconditionally when dictation is enabled (feeds sidebar tray)
  useEffect(() => {
    if (dictationEnabled) {
      loadDictationHistory();
    }
  }, [dictationEnabled, loadDictationHistory]);

  // Auto-refresh dictation history after new dictation completes
  useEffect(() => {
    if (!dictationEnabled) return;
    const handler = () => {
      setTimeout(() => loadDictationHistory(), 300);
    };
    window.addEventListener("yapstack:dictation-idle", handler);
    return () => window.removeEventListener("yapstack:dictation-idle", handler);
  }, [dictationEnabled, loadDictationHistory]);

  // Derive chat context from navigation state
  const chatContext: ChatContext | null = useMemo(() => {
    if (currentView === "settings") return null;
    if (currentView === "note-detail" && selectedSessionId) {
      return { type: "session", sessionId: selectedSessionId };
    }
    // note-list view
    switch (listFilter.type) {
      case "folder":
        return listFilter.folderId
          ? { type: "folder", folderId: listFilter.folderId }
          : { type: "all" };
      case "pinned":
        return { type: "pinned" };
      case "dictation":
        return { type: "dictation" };
      default:
        return { type: "all" };
    }
  }, [currentView, selectedSessionId, listFilter]);

  const handleDragStart = useCallback((event: DragStartEvent) => {
    const data = event.active.data.current;
    if (data?.type === "session") {
      setActiveSession({
        id: data.sessionId,
        title: data.title ?? "Untitled",
        sessionType: data.sessionType ?? "recording",
        isPinned: data.isPinned ?? false,
        totalSegments: data.totalSegments ?? 0,
        createdAt: data.createdAt ?? "",
      });
    } else if (data?.type === "folder") {
      setActiveFolder({
        id: data.folderId,
        name: data.folderName ?? "Folder",
      });
    }
  }, []);

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      setActiveSession(null);
      setActiveFolder(null);
      const { active, over } = event;
      if (!over) return;

      const activeData = active.data.current;
      const overData = over.data.current;

      if (
        activeData?.type === "session" &&
        overData?.type === "folder"
      ) {
        addSessionToFolder(activeData.sessionId, overData.folderId);
      } else if (
        activeData?.type === "folder" &&
        overData?.type === "folder"
      ) {
        const draggedId: string = activeData.folderId;
        const targetId: string = overData.folderId;

        // Prevent dropping onto self
        if (draggedId === targetId) return;

        // Same parent → reorder siblings
        if (activeData.parentId === overData.parentId) {
          reorderFolders(draggedId, targetId);
        } else {
          // Different parent → reparent
          // Prevent circular reference (dropping parent into its own descendant)
          if (isDescendantOf(folders, targetId, draggedId)) {
            toast.error("Cannot move a folder into its own subfolder");
            return;
          }

          moveFolder(draggedId, targetId);
          const targetName = folders.find((f) => f.id === targetId)?.name;
          toast.success(`Moved to ${targetName ?? "folder"}`);
        }
      }
    },
    [addSessionToFolder, moveFolder, reorderFolders, folders],
  );

  const mainContent = (
    <main className="relative flex flex-1 flex-col min-w-0 h-full">
      <SetupBanner />
      {currentView === "settings" && <SettingsPanel />}
      {currentView === "note-detail" && <NoteDetailView />}
      {currentView === "note-list" && <NoteCardList />}
      {chatContext && chatContext.type !== "session" && (
        <ListContextBar chatContext={chatContext} />
      )}
    </main>
  );

  return (
    <>
    <OnboardingModal />
    <TooltipProvider>
      <DndContext sensors={sensors} onDragStart={handleDragStart} onDragEnd={handleDragEnd}>
        <div className="flex h-full flex-col select-none">
          <TitleBar />
          <ResizablePanelGroup
            key={sidebarCollapsed ? "collapsed" : "expanded"}
            orientation="horizontal"
            className="flex-1 min-h-0"
          >
            {!sidebarCollapsed && (
              <>
                <ResizablePanel defaultSize="20%" minSize="15%" maxSize="30%">
                  <AppSidebar />
                </ResizablePanel>
                <ResizableHandle />
              </>
            )}
            <ResizablePanel defaultSize={sidebarCollapsed ? "100%" : "80%"}>
              {mainContent}
            </ResizablePanel>
          </ResizablePanelGroup>
        </div>
        <DragOverlay dropAnimation={null}>
          {activeSession && (
            <div className="flex w-56 flex-col gap-0.5 rounded-lg border bg-card px-2 py-2 shadow-lg">
              <div className="flex min-w-0 items-center gap-2">
                {activeSession.sessionType === "manual" ? (
                  <PenLine className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                ) : (
                  <Mic className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                )}
                <span className="truncate text-sm font-medium">
                  {activeSession.title || "Untitled"}
                </span>
                {activeSession.isPinned && (
                  <Pin className="h-3 w-3 shrink-0 text-muted-foreground" />
                )}
              </div>
              <div className="flex items-center gap-2 text-[11px] text-muted-foreground pl-[22px]">
                {activeSession.totalSegments > 0 && (
                  <span>{activeSession.totalSegments} segments</span>
                )}
              </div>
            </div>
          )}
          {activeFolder && (
            <div className="flex w-40 items-center gap-2 rounded-lg border bg-card px-2 py-1.5 shadow-lg">
              <Folder className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span className="truncate text-xs font-medium">
                {activeFolder.name}
              </span>
            </div>
          )}
        </DragOverlay>
      </DndContext>
    </TooltipProvider>
    </>
  );
}
