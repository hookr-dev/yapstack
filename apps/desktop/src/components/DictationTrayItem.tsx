import { useState, useRef } from "react";
import { useAppStore } from "@/stores/appStore";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Play, Pause, Copy, FileText, Trash2 } from "lucide-react";
import {
  createManualSession as dbCreateManualSession,
  saveNote,
  updateDictationHistorySessionId,
} from "@/lib/db";
import { toast } from "sonner";
import { formatRelativeTime } from "@/lib/utils";
import type { DbDictationHistory } from "@/lib/db";

export function DictationTrayItem({ entry }: { entry: DbDictationHistory }) {
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
    const audio = new Audio(`audio-stream://localhost/${entry.id}.wav`);
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

  const handleDelete = () => {
    deleteEntry(entry.id);
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <button
          className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-sidebar-accent/50 cursor-default"
          onClick={handleCopy}
          title={entry.output_text}
        >
          <span className="flex-1 truncate text-left">
            {entry.output_text}
          </span>
          <span className="shrink-0 text-[10px] tabular-nums opacity-60">
            {formatRelativeTime(entry.created_at)}
          </span>
        </button>
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
        {!entry.session_id && (
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
