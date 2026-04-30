import { useState, useRef, useEffect } from "react";
import { useAppStore } from "@/stores/appStore";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ArrowLeft, Check, FolderMinus, FolderOpen, FolderPlus, Loader2, Mic, MoreHorizontal, Square, Trash2 } from "lucide-react";
import type { DbSession } from "@/lib/db";
import {
  canResumeSession,
  updateSessionTitle,
  getSession,
} from "@/lib/db";
import { formatDuration, formatElapsed } from "@/lib/utils";
import { ICON_MAP } from "@/lib/folder-constants";
import type { FolderTreeNode } from "@/lib/folder-tree";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

function RecordingBadge() {
  const activeSessionStartTime = useAppStore((s) => s.activeSessionStartTime);
  const stopActiveSession = useAppStore((s) => s.stopActiveSession);
  const sessionStopping = useAppStore((s) => s.sessionStopping);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    if (!activeSessionStartTime || sessionStopping) return;
    const update = () => setElapsed(Date.now() - activeSessionStartTime);
    update();
    const id = setInterval(update, 1000);
    return () => clearInterval(id);
  }, [activeSessionStartTime, sessionStopping]);

  if (sessionStopping) {
    return (
      <div
        className="flex items-center gap-2 rounded-full bg-amber-500/10 border border-amber-500/20 text-amber-600 dark:text-amber-400 px-3 py-1 text-xs font-medium animate-pulse"
        aria-live="polite"
      >
        <Loader2 className="h-3 w-3 animate-spin" />
        <span>Finalizing…</span>
      </div>
    );
  }

  return (
    <button
      onClick={stopActiveSession}
      className="flex items-center gap-2 rounded-full bg-destructive/10 border border-destructive/20 text-destructive px-3 py-1 text-xs font-medium hover:bg-destructive/20 transition-colors"
    >
      <span className="h-2 w-2 rounded-full bg-destructive animate-pulse" />
      <span className="font-mono">{formatElapsed(elapsed)}</span>
      <Square className="h-3 w-3" />
    </button>
  );
}

function FolderMenuNode({
  node,
  sessionId,
  sessionFolderIds,
  onToggle,
}: {
  node: FolderTreeNode;
  sessionId: string;
  sessionFolderIds: string[];
  onToggle: (sessionId: string, folderId: string) => void;
}) {
  const { folder, children } = node;
  const isInFolder = sessionFolderIds.includes(folder.id);
  const FolderIcon = folder.icon ? ICON_MAP[folder.icon] : null;

  if (children.length === 0) {
    return (
      <DropdownMenuItem onClick={() => onToggle(sessionId, folder.id)}>
        {FolderIcon && (
          <FolderIcon
            style={folder.color ? { color: folder.color } : undefined}
          />
        )}
        <span className="flex-1">{folder.name}</span>
        {isInFolder && <Check className="text-muted-foreground" />}
      </DropdownMenuItem>
    );
  }

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        {FolderIcon && (
          <FolderIcon
            style={folder.color ? { color: folder.color } : undefined}
          />
        )}
        <span className="flex-1">{folder.name}</span>
        {isInFolder && <Check className="text-muted-foreground" />}
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent>
        <DropdownMenuItem onClick={() => onToggle(sessionId, folder.id)}>
          {FolderIcon && (
            <FolderIcon
              style={folder.color ? { color: folder.color } : undefined}
            />
          )}
          <span className="flex-1">{folder.name}</span>
          {isInFolder && <Check className="text-muted-foreground" />}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        {children.map((child) => (
          <FolderMenuNode
            key={child.folder.id}
            node={child}
            sessionId={sessionId}
            sessionFolderIds={sessionFolderIds}
            onToggle={onToggle}
          />
        ))}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}

export function SessionHeader({ session }: { session: DbSession }) {
  const deleteSession = useAppStore((s) => s.deleteSession);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const navigateTo = useAppStore((s) => s.navigateTo);
  const loadSessions = useAppStore((s) => s.loadSessions);
  const resumeSession = useAppStore((s) => s.resumeSession);
  const stopActiveSession = useAppStore((s) => s.stopActiveSession);
  const liveTranscriptionActive = useAppStore((s) => s.liveTranscriptionActive);
  const sessionStopping = useAppStore((s) => s.sessionStopping);
  const viewSessionParts = useAppStore((s) => s.viewSessionParts);
  const folderTree = useAppStore((s) => s.folderTree);
  const sessionFolderMap = useAppStore((s) => s.sessionFolderMap);
  const toggleSessionFolder = useAppStore((s) => s.toggleSessionFolder);
  const removeSessionFromAllFolders = useAppStore((s) => s.removeSessionFromAllFolders);

  const sessionFolderIds = sessionFolderMap[session.id] ?? [];
  const inAnyFolder = sessionFolderIds.length > 0;
  const isRecording = session.id === activeSessionId;
  const [isEditingTitle, setIsEditingTitle] = useState(false);
  const [titleText, setTitleText] = useState(session.title);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);

  // Sync local titleText when session prop changes (e.g. after AI tool updates title)
  const prevTitleRef = useRef(session.title);
  if (session.title !== prevTitleRef.current) {
    prevTitleRef.current = session.title;
    if (!isEditingTitle) {
      setTitleText(session.title);
    }
  }

  // Listen for keyboard shortcut delete trigger
  useEffect(() => {
    const handler = () => {
      if (!isRecording) setDeleteDialogOpen(true);
    };
    window.addEventListener("yapstack:confirm-delete-session", handler);
    return () => window.removeEventListener("yapstack:confirm-delete-session", handler);
  }, [isRecording]);

  const handleSaveTitle = async () => {
    const trimmed = titleText.trim();
    if (trimmed && trimmed !== session.title) {
      await updateSessionTitle(session.id, trimmed);
      await loadSessions();
      // Refresh viewSession so the header re-renders with the new title
      const updated = await getSession(session.id);
      if (updated) {
        useAppStore.setState({ viewSession: updated });
      }
    }
    setIsEditingTitle(false);
  };

  return (
    <div className="flex items-center justify-between border-b px-4 py-2">
      {/* Left: back + title */}
      <div className="flex items-center gap-2 min-w-0 flex-1">
        <button
          className="rounded p-1 text-muted-foreground hover:bg-muted shrink-0"
          onClick={() => navigateTo("note-list")}
          aria-label="Back to notes"
        >
          <ArrowLeft className="h-4 w-4" />
        </button>

        {isEditingTitle ? (
          <Input
            value={titleText}
            onChange={(e) => setTitleText(e.target.value)}
            onBlur={handleSaveTitle}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSaveTitle();
              if (e.key === "Escape") setIsEditingTitle(false);
            }}
            className="h-7 text-sm font-medium"
            autoFocus
          />
        ) : (
          <span
            className="truncate text-sm font-medium cursor-pointer hover:underline"
            onDoubleClick={() => {
              if (!isRecording) {
                setTitleText(session.title);
                setIsEditingTitle(true);
              }
            }}
          >
            {session.title || "Untitled"}
          </span>
        )}
      </div>

      {/* Center: badges + info */}
      <div className="flex items-center gap-2 shrink-0 px-2">
        {isRecording ? (
          <RecordingBadge />
        ) : (
          <>
            {session.duration_seconds != null && (
              <span className="text-xs text-muted-foreground">
                {formatDuration(session.duration_seconds)}
              </span>
            )}
            {session.total_segments > 0 && (
              <span className="text-xs text-muted-foreground">
                &middot; {session.total_segments} segments
              </span>
            )}
          </>
        )}
      </div>

      {/* Right: dropdown actions */}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-xs" className="shrink-0">
            <MoreHorizontal className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {!isRecording && canResumeSession(session, viewSessionParts, liveTranscriptionActive, sessionStopping) && (
            <DropdownMenuItem onClick={() => resumeSession(session.id)}>
              <Mic />
              Resume recording
            </DropdownMenuItem>
          )}
          {!isRecording && viewSessionParts.length > 0 && (
            <DropdownMenuItem
              onClick={() => {
                // Reveal the most recent part. The other parts live in the
                // same directory, so this gets the user there either way.
                const latest = viewSessionParts[viewSessionParts.length - 1];
                if (latest) {
                  revealItemInDir(latest.file_path).catch((e) =>
                    console.error("Failed to reveal file:", e),
                  );
                }
              }}
            >
              <FolderOpen />
              Show audio file
            </DropdownMenuItem>
          )}
          {folderTree.length > 0 && (
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <FolderPlus />
                <span className="flex-1">Folders</span>
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                {folderTree.map((node) => (
                  <FolderMenuNode
                    key={node.folder.id}
                    node={node}
                    sessionId={session.id}
                    sessionFolderIds={sessionFolderIds}
                    onToggle={toggleSessionFolder}
                  />
                ))}
                {inAnyFolder && (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      onClick={() => removeSessionFromAllFolders(session.id)}
                    >
                      <FolderMinus />
                      Remove from all folders
                    </DropdownMenuItem>
                  </>
                )}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          )}
          <DropdownMenuSeparator />
          {isRecording ? (
            <DropdownMenuItem
              className="text-destructive"
              disabled={sessionStopping}
              onClick={stopActiveSession}
            >
              <Square />
              Stop recording
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => setDeleteDialogOpen(true)}
            >
              <Trash2 />
              Delete session
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>

      <AlertDialog open={deleteDialogOpen} onOpenChange={setDeleteDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete session?</AlertDialogTitle>
            <AlertDialogDescription>
              This will permanently delete this session and its transcriptions.
              This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-white hover:bg-destructive/90"
              onClick={() => deleteSession(session.id)}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
