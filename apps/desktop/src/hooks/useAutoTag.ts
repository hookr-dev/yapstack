import { useState, useEffect, useRef, useCallback } from "react";
import { useAppStore } from "@/stores/appStore";
import {
  FolderSuggestionTracker,
  buildFolderProfiles,
  type FolderSuggestion,
} from "@/lib/auto-tag";
import { commands } from "@/lib/tauri";
import { listFolders, listTags } from "@/lib/db";
import { buildVocabularyHints } from "@/lib/transcription";

export function useAutoTag(sessionId: string | null, isRecording: boolean) {
  const folders = useAppStore((s) => s.folders);
  const tags = useAppStore((s) => s.tags);
  const sessionFolderMap = useAppStore((s) => s.sessionFolderMap);
  const sessionTagMap = useAppStore((s) => s.sessionTagMap);
  const activeSessionSegments = useAppStore((s) => s.activeSessionSegments);
  const addSessionToFolder = useAppStore((s) => s.addSessionToFolder);

  const [suggestions, setSuggestions] = useState<FolderSuggestion[]>([]);
  const trackerRef = useRef<FolderSuggestionTracker | null>(null);
  const processedCountRef = useRef(0);
  const profilesRef = useRef(
    buildFolderProfiles(folders, tags, sessionFolderMap, sessionTagMap),
  );

  // Profiles depend on the global folder/tag membership graph; the runtime
  // tracker just consumes them. Rebuild when any of those inputs change so a
  // newly-created folder or a tag added mid-session is reflected.
  useEffect(() => {
    profilesRef.current = buildFolderProfiles(
      folders,
      tags,
      sessionFolderMap,
      sessionTagMap,
    );
  }, [folders, tags, sessionFolderMap, sessionTagMap]);

  useEffect(() => {
    if (!sessionId || !isRecording) {
      trackerRef.current = null;
      processedCountRef.current = 0;
      setSuggestions([]);
      return;
    }
    const existingFolders = sessionFolderMap[sessionId] ?? [];
    trackerRef.current = new FolderSuggestionTracker(
      profilesRef.current,
      existingFolders,
    );
    processedCountRef.current = 0;
  }, [sessionId, isRecording, sessionFolderMap]);

  useEffect(() => {
    if (!trackerRef.current || !isRecording) return;

    const newSegments = activeSessionSegments.slice(processedCountRef.current);
    if (newSegments.length === 0) return;
    processedCountRef.current = activeSessionSegments.length;

    let latest: FolderSuggestion[] = [];
    for (const seg of newSegments) {
      latest = trackerRef.current.processSegment(seg.text);
    }
    // The tracker now returns the current top recommendation (0 or 1 item)
    // each call, so replace state rather than append to it.
    setSuggestions((prev) => {
      if (latest.length === 0 && prev.length === 0) return prev;
      if (
        latest.length === prev.length &&
        latest.every((s, i) => s.id === prev[i].id && s.confidence === prev[i].confidence)
      ) {
        return prev;
      }
      return latest;
    });
  }, [activeSessionSegments, isRecording]);

  const acceptSuggestion = useCallback(
    async (suggestion: FolderSuggestion) => {
      if (!sessionId) return;
      trackerRef.current?.accept(suggestion.id);
      setSuggestions((prev) => prev.filter((s) => s.id !== suggestion.id));
      await addSessionToFolder(sessionId, suggestion.id);

      const updatedHints = buildVocabularyHints(await listFolders(), await listTags());
      if (updatedHints) {
        commands.updateVocabularyHints(updatedHints).catch(() => {});
      }
    },
    [sessionId, addSessionToFolder],
  );

  const dismissSuggestion = useCallback((suggestion: FolderSuggestion) => {
    trackerRef.current?.dismiss(suggestion.id);
    setSuggestions((prev) => prev.filter((s) => s.id !== suggestion.id));
  }, []);

  return { suggestions, acceptSuggestion, dismissSuggestion };
}
