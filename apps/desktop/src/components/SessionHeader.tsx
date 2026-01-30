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
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ArrowLeft, FolderOpen, MoreHorizontal, Square, Trash2 } from "lucide-react";
import type { DbSession } from "@/lib/db";
import { updateSessionTitle, getSession } from "@/lib/db";
import { formatDuration, formatElapsed } from "@/lib/utils";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

function RecordingBadge() {
  const activeSessionStartTime = useAppStore((s) => s.activeSessionStartTime);
  const stopActiveSession = useAppStore((s) => s.stopActiveSession);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    if (!activeSessionStartTime) return;
    const update = () => setElapsed(Date.now() - activeSessionStartTime);
    update();
    const id = setInterval(update, 1000);
    return () => clearInterval(id);
  }, [activeSessionStartTime]);

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

export function SessionHeader({ session }: { session: DbSession }) {
  const deleteSession = useAppStore((s) => s.deleteSession);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const navigateTo = useAppStore((s) => s.navigateTo);
  const loadSessions = useAppStore((s) => s.loadSessions);

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
      {!isRecording && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon-xs" className="shrink-0">
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {session.wav_file_path && (
              <DropdownMenuItem
                onClick={() => {
                  revealItemInDir(session.wav_file_path!).catch((e) =>
                    console.error("Failed to reveal file:", e),
                  );
                }}
              >
                <FolderOpen />
                Show audio file
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => setDeleteDialogOpen(true)}
            >
              <Trash2 />
              Delete session
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      )}

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
