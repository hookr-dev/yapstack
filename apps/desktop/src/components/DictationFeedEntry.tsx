import { useState, useRef } from "react";
import { useAppStore } from "@/stores/appStore";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Bot, Play, Pause, Trash2, Copy, FileText } from "lucide-react";
import {
  createManualSession as dbCreateManualSession,
  saveNote,
  updateDictationHistorySessionId,
} from "@/lib/db";
import { toast } from "sonner";
import type { DbDictationHistory } from "@/lib/db";

function formatTime12h(dateStr: string): string {
  const date = new Date(dateStr + "Z");
  return date.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
    hour12: true,
  });
}

function formatDurationCompact(seconds: number): string {
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const mins = Math.floor(seconds / 60);
  const secs = Math.round(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

export function DictationFeedEntry({ entry }: { entry: DbDictationHistory }) {
  const deleteEntry = useAppStore((s) => s.deleteDictationHistoryEntry);
  const loadSessions = useAppStore((s) => s.loadSessions);
  const openSession = useAppStore((s) => s.openSession);
  const loadDictationHistory = useAppStore((s) => s.loadDictationHistory);

  const [playing, setPlaying] = useState(false);
  const audioRef = useRef<HTMLAudioElement | null>(null);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(entry.output_text);
      toast.success("Copied to clipboard");
    } catch {
      toast.error("Failed to copy");
    }
  };

  const handlePlayAudio = () => {
    if (!entry.wav_file_path) return;
    if (playing && audioRef.current) {
      audioRef.current.pause();
      audioRef.current = null;
      setPlaying(false);
      return;
    }
    const ext = entry.wav_file_path?.endsWith(".mp3") ? "mp3" : "wav";
    const audio = new Audio(`audio-stream://localhost/${entry.id}.${ext}`);
    audio.onended = () => {
      setPlaying(false);
      audioRef.current = null;
    };
    audio.onerror = () => {
      setPlaying(false);
      audioRef.current = null;
    };
    audioRef.current = audio;
    audio.play();
    setPlaying(true);
  };

  const handleMoveToNote = async () => {
    try {
      const sessionId = crypto.randomUUID();
      const title = entry.output_text.slice(0, 60);
      await dbCreateManualSession(sessionId, title);
      await saveNote(sessionId, `<p>${entry.output_text}</p>`);
      await updateDictationHistorySessionId(entry.id, sessionId);
      await loadSessions();
      await loadDictationHistory();
      await openSession(sessionId);
      toast.success("Moved to note");
    } catch (e) {
      console.error("Failed to move to note:", e);
      toast.error("Failed to create note");
    }
  };

  const handleOpenNote = () => {
    if (entry.session_id) {
      openSession(entry.session_id);
    }
  };

  const handleDelete = () => {
    deleteEntry(entry.id);
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div className="px-4 py-3 border-b border-border/30 hover:bg-accent/30 transition-colors cursor-default">
          {/* Metadata line */}
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-muted-foreground tabular-nums">
              {formatTime12h(entry.created_at)}
            </span>
            <span className="text-muted-foreground/40">·</span>
            <span className="rounded-full bg-muted px-1.5 py-0.5 text-[10px] font-medium truncate max-w-24">
              {entry.slot_name}
            </span>
            {entry.ai_enabled === 1 && (
              <>
                <span className="text-muted-foreground/40">·</span>
                <span className="inline-flex items-center gap-0.5 rounded-full bg-purple-500/10 px-1.5 py-0.5 text-[10px] font-medium text-purple-500">
                  <Bot className="h-2.5 w-2.5" />
                  AI
                </span>
              </>
            )}
            {entry.session_id && (
              <>
                <span className="text-muted-foreground/40">·</span>
                <button
                  onClick={handleOpenNote}
                  className="inline-flex items-center gap-0.5 rounded-full bg-blue-500/10 px-1.5 py-0.5 text-[10px] font-medium text-blue-500 hover:bg-blue-500/20 transition-colors"
                >
                  <FileText className="h-2.5 w-2.5" />
                  Note
                </button>
              </>
            )}
            <div className="flex-1" />
            {entry.wav_duration_seconds != null && (
              <span className="text-[11px] text-muted-foreground tabular-nums mr-0.5">
                {formatDurationCompact(entry.wav_duration_seconds)}
              </span>
            )}
            {entry.wav_file_path && (
              <button
                onClick={handlePlayAudio}
                className={`rounded p-1 transition-colors ${playing ? "text-primary hover:text-primary/80 hover:bg-accent" : "text-muted-foreground hover:text-foreground hover:bg-accent"}`}
                aria-label={playing ? "Pause audio" : "Play audio"}
              >
                {playing ? (
                  <Pause className="h-3 w-3" />
                ) : (
                  <Play className="h-3 w-3" />
                )}
              </button>
            )}
            {entry.session_id ? (
              <button
                onClick={handleOpenNote}
                className="rounded p-1 text-blue-500 hover:text-blue-400 hover:bg-accent transition-colors"
                aria-label="Open note"
              >
                <FileText className="h-3 w-3" />
              </button>
            ) : (
              <button
                onClick={handleMoveToNote}
                className="rounded p-1 text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
                aria-label="Move to note"
              >
                <FileText className="h-3 w-3" />
              </button>
            )}
            <button
              onClick={handleCopy}
              className="rounded p-1 text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
              aria-label="Copy"
            >
              <Copy className="h-3 w-3" />
            </button>
            <button
              onClick={handleDelete}
              className="rounded p-1 text-muted-foreground hover:text-destructive hover:bg-accent transition-colors"
              aria-label="Delete"
            >
              <Trash2 className="h-3 w-3" />
            </button>
          </div>

          {/* Text body — full text, no truncation */}
          <p className="mt-1.5 text-xs leading-relaxed text-foreground">
            {entry.output_text}
          </p>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onClick={handleCopy}>
          <Copy className="h-3.5 w-3.5 mr-2" />
          Copy Text
        </ContextMenuItem>
        {entry.wav_file_path && (
          <ContextMenuItem onClick={handlePlayAudio}>
            {playing ? (
              <Pause className="h-3.5 w-3.5 mr-2" />
            ) : (
              <Play className="h-3.5 w-3.5 mr-2" />
            )}
            {playing ? "Pause Audio" : "Play Audio"}
          </ContextMenuItem>
        )}
        {entry.session_id ? (
          <ContextMenuItem onClick={handleOpenNote}>
            <FileText className="h-3.5 w-3.5 mr-2" />
            Open Note
          </ContextMenuItem>
        ) : (
          <ContextMenuItem onClick={handleMoveToNote}>
            <FileText className="h-3.5 w-3.5 mr-2" />
            Move to Note
          </ContextMenuItem>
        )}
        <ContextMenuItem onClick={handleDelete} className="text-destructive">
          <Trash2 className="h-3.5 w-3.5 mr-2" />
          Delete
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
