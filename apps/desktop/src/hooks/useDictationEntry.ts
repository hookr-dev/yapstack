import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useAppStore } from "@/stores/appStore";
import {
  createManualSession as dbCreateManualSession,
  saveNote,
  updateDictationHistorySessionId,
  type DbDictationHistory,
} from "@/lib/db";

/// Shared state and handlers for any UI surface that renders a single
/// `DictationHistory` entry. Owns the play/pause audio toggle, clipboard
/// copy, "move to note" (creates a manual session bound to the entry), and
/// delete. The audio element is paused and released on unmount so clicking
/// play and then navigating away doesn't leak playback.
export function useDictationEntry(entry: DbDictationHistory) {
  const deleteEntry = useAppStore((s) => s.deleteDictationHistoryEntry);
  const loadSessions = useAppStore((s) => s.loadSessions);
  const openSession = useAppStore((s) => s.openSession);
  const loadDictationHistory = useAppStore((s) => s.loadDictationHistory);

  const [playing, setPlaying] = useState(false);
  const audioRef = useRef<HTMLAudioElement | null>(null);

  // Pause + release on unmount.
  useEffect(() => {
    return () => {
      const audio = audioRef.current;
      if (audio) {
        audio.pause();
        audio.src = "";
        audioRef.current = null;
      }
    };
  }, []);

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
    setPlaying(true);
    // `audio.play()` returns a Promise that rejects if playback was
    // interrupted (e.g. user clicked stop before it started). Swallow it so
    // the rejection doesn't surface as an unhandled-promise warning, and
    // unwind the playing flag if the start itself failed.
    audio.play().catch(() => {
      if (audioRef.current === audio) {
        audioRef.current = null;
        setPlaying(false);
      }
    });
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

  return {
    playing,
    handleCopy,
    handlePlayAudio,
    handleMoveToNote,
    handleOpenNote,
    handleDelete,
  };
}
