import { type CSSProperties, memo, useMemo } from "react";
import { useAppStore } from "@/stores/appStore";
import type { DbSession } from "@/lib/db";
import { cn, formatRelativeTime } from "@/lib/utils";
import { Pin, PinOff, Mic, PenLine, Check, ExternalLink, FolderMinus, Trash2 } from "lucide-react";
import { ICON_MAP } from "@/lib/folder-constants";
import { useDraggable } from "@dnd-kit/core";
import { getDisplayFolders, type FolderTreeNode } from "@/lib/folder-tree";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubTrigger,
  ContextMenuSubContent,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

function folderBadgeStyle(color: string | null): CSSProperties {
  if (!color) return {};
  return {
    backgroundColor: `color-mix(in oklch, ${color} 15%, transparent)`,
    color: color,
  };
}

function FolderNodeItem({
  folder,
  isInFolder,
  onClick,
}: {
  folder: FolderTreeNode["folder"];
  isInFolder: boolean;
  onClick: () => void;
}) {
  const FolderIcon = folder.icon ? ICON_MAP[folder.icon] : null;
  return (
    <ContextMenuItem onClick={onClick}>
      {FolderIcon && (
        <FolderIcon
          style={folder.color ? { color: folder.color } : undefined}
        />
      )}
      <span className="flex-1">{folder.name}</span>
      {isInFolder && <Check className="text-muted-foreground" />}
    </ContextMenuItem>
  );
}

function FolderNode({
  node,
  sessionId,
  sessionFolderIds,
  toggleSessionFolder,
}: {
  node: FolderTreeNode;
  sessionId: string;
  sessionFolderIds: string[];
  toggleSessionFolder: (sessionId: string, folderId: string) => void;
}) {
  const { folder, children } = node;
  const isInFolder = sessionFolderIds.includes(folder.id);
  const FolderIcon = folder.icon ? ICON_MAP[folder.icon] : null;

  if (children.length === 0) {
    return (
      <FolderNodeItem
        folder={folder}
        isInFolder={isInFolder}
        onClick={() => toggleSessionFolder(sessionId, folder.id)}
      />
    );
  }

  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger>
        {FolderIcon && (
          <FolderIcon
            style={folder.color ? { color: folder.color } : undefined}
          />
        )}
        <span className="flex-1">{folder.name}</span>
        {isInFolder && <Check className="text-muted-foreground" />}
      </ContextMenuSubTrigger>
      <ContextMenuSubContent>
        <FolderNodeItem
          folder={folder}
          isInFolder={isInFolder}
          onClick={() => toggleSessionFolder(sessionId, folder.id)}
        />
        <ContextMenuSeparator />
        {children.map((child) => (
          <FolderNode
            key={child.folder.id}
            node={child}
            sessionId={sessionId}
            sessionFolderIds={sessionFolderIds}
            toggleSessionFolder={toggleSessionFolder}
          />
        ))}
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

export const NoteCard = memo(function NoteCard({ session }: { session: DbSession }) {
  const selectedSessionId = useAppStore((s) => s.selectedSessionId);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const openSession = useAppStore((s) => s.openSession);
  const deleteSession = useAppStore((s) => s.deleteSession);
  const togglePin = useAppStore((s) => s.togglePin);
  const folders = useAppStore((s) => s.folders);
  const folderTree = useAppStore((s) => s.folderTree);
  const folderByIdMap = useAppStore((s) => s.folderByIdMap);
  const sessionFolderMap = useAppStore((s) => s.sessionFolderMap);
  const toggleSessionFolder = useAppStore((s) => s.toggleSessionFolder);
  const removeSessionFromAllFolders = useAppStore((s) => s.removeSessionFromAllFolders);

  const listFilter = useAppStore((s) => s.listFilter);

  const sessionFolderIds = useMemo(
    () => sessionFolderMap[session.id] ?? [],
    [sessionFolderMap, session.id],
  );
  const inAnyFolder = sessionFolderIds.length > 0;

  const displayFolders = useMemo(() => {
    const contextFolderId = listFilter.type === "folder" ? (listFilter.folderId ?? null) : null;
    return getDisplayFolders(sessionFolderIds, folders, contextFolderId, folderByIdMap);
  }, [folders, folderByIdMap, sessionFolderIds, listFilter]);

  const { attributes, listeners, setNodeRef, isDragging } =
    useDraggable({
      id: `session-${session.id}`,
      data: {
        type: "session",
        sessionId: session.id,
        title: session.title,
        sessionType: session.session_type,
        isPinned: session.is_pinned === 1,
        totalSegments: session.total_segments,
        createdAt: session.created_at,
      },
    });

  const isSelected = selectedSessionId === session.id;
  const isRecording =
    session.id === activeSessionId || session.status === "recording";
  const isPinned = session.is_pinned === 1;
  const isManual = session.session_type === "manual";
  const TypeIcon = isManual ? PenLine : Mic;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          ref={setNodeRef}
          {...listeners}
          {...attributes}
          onClick={() => openSession(session.id)}
          className={cn(
            "group flex w-full items-start gap-2 rounded-lg border px-2 py-2.5 text-left transition-all hover:shadow-sm",
            isSelected
              ? "border-ring bg-accent"
              : "border-transparent hover:bg-muted/50",
            isDragging && "opacity-50 border-border",
          )}
        >
          <div className="flex flex-1 min-w-0 flex-col gap-1">
            <div className="flex min-w-0 items-center gap-2">
              {isRecording && (
                <span
                  className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-destructive"
                  aria-hidden
                />
              )}
              <TypeIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span className="truncate text-sm font-medium">
                {session.title || "Untitled"}
              </span>
              {isPinned && (
                <Pin className="h-3 w-3 shrink-0 text-muted-foreground" />
              )}
            </div>
            <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
              <span>{formatRelativeTime(session.created_at)}</span>
              {session.total_segments > 0 && (
                <span>&middot; {session.total_segments} segments</span>
              )}
            </div>
          </div>
          {displayFolders.length > 0 && (
            <div className="flex shrink-0 items-center gap-1 self-center">
              {displayFolders.map((f) => {
                const Icon = f.icon ? ICON_MAP[f.icon] : null;
                return (
                  <Tooltip key={f.id}>
                    <TooltipTrigger asChild>
                      <span
                        className={cn(
                          "inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10px] font-medium transition-colors",
                          f.color ? "bg-muted/50" : "bg-muted text-muted-foreground",
                        )}
                        style={folderBadgeStyle(f.color)}
                      >
                        {Icon ? (
                          <Icon className="h-2.5 w-2.5 shrink-0" />
                        ) : (
                          <span
                            className="h-1.5 w-1.5 rounded-full shrink-0"
                            style={{ backgroundColor: f.color ?? "var(--muted-foreground)" }}
                          />
                        )}
                        <span className="truncate max-w-[72px]">{f.name}</span>
                      </span>
                    </TooltipTrigger>
                    <TooltipContent side="bottom">{f.name}</TooltipContent>
                  </Tooltip>
                );
              })}
            </div>
          )}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onClick={() => openSession(session.id)}>
          <ExternalLink />
          Open
        </ContextMenuItem>
        <ContextMenuItem onClick={() => togglePin(session.id)}>
          {isPinned ? (
            <PinOff />
          ) : (
            <Pin />
          )}
          {isPinned ? "Unpin" : "Pin"}
        </ContextMenuItem>
        {folderTree.length > 0 && (
          <>
            <ContextMenuSeparator />
            {folderTree.map((node) => (
              <FolderNode
                key={node.folder.id}
                node={node}
                sessionId={session.id}
                sessionFolderIds={sessionFolderIds}
                toggleSessionFolder={toggleSessionFolder}
              />
            ))}
            {inAnyFolder && (
              <>
                <ContextMenuSeparator />
                <ContextMenuItem
                  onClick={() => removeSessionFromAllFolders(session.id)}
                >
                  <FolderMinus />
                  Remove from all folders
                </ContextMenuItem>
              </>
            )}
          </>
        )}
        <ContextMenuSeparator />
        <ContextMenuItem
          className="text-destructive"
          disabled={isRecording}
          onClick={() => deleteSession(session.id)}
        >
          <Trash2 />
          Delete
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
});
