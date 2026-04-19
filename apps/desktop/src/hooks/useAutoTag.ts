import { useState, useEffect, useRef, useCallback } from "react";
import { useAppStore } from "@/stores/appStore";
import {
  FolderSuggestionTracker,
  buildFolderKeywordMap,
  type FolderSuggestion,
} from "@/lib/auto-tag";
import { commands } from "@/lib/tauri";
import { listFolders, listTags } from "@/lib/db";
import { buildVocabularyHints } from "@/lib/transcription";

export function useAutoTag(sessionId: string | null, isRecording: boolean) {
  const folders = useAppStore((s) => s.folders);
  const sessionFolderMap = useAppStore((s) => s.sessionFolderMap);
  const activeSessionSegments = useAppStore((s) => s.activeSessionSegments);
  const addSessionToFolder = useAppStore((s) => s.addSessionToFolder);

  const [suggestions, setSuggestions] = useState<FolderSuggestion[]>([]);
  const trackerRef = useRef<FolderSuggestionTracker | null>(null);
  const processedCountRef = useRef(0);
  const keywordMapRef = useRef(buildFolderKeywordMap(folders));

  useEffect(() => {
    keywordMapRef.current = buildFolderKeywordMap(folders);
  }, [folders]);

  useEffect(() => {
    if (!sessionId || !isRecording) {
      trackerRef.current = null;
      processedCountRef.current = 0;
      setSuggestions([]);
      return;
    }
    const existingFolders = sessionFolderMap[sessionId] ?? [];
    trackerRef.current = new FolderSuggestionTracker(existingFolders);
    processedCountRef.current = 0;
  }, [sessionId, isRecording, sessionFolderMap]);

  useEffect(() => {
    if (!trackerRef.current || !isRecording) return;

    const newSegments = activeSessionSegments.slice(processedCountRef.current);
    if (newSegments.length === 0) return;
    processedCountRef.current = activeSessionSegments.length;

    for (const seg of newSegments) {
      const newSuggestions = trackerRef.current.processSegment(
        seg.text,
        keywordMapRef.current,
      );
      if (newSuggestions.length > 0) {
        setSuggestions((prev) => {
          const existingIds = new Set(prev.map((s) => s.id));
          const unique = newSuggestions.filter((s) => !existingIds.has(s.id));
          return unique.length > 0 ? [...prev, ...unique] : prev;
        });
      }
    }
  }, [activeSessionSegments, isRecording]);

  const acceptSuggestion = useCallback(
    async (suggestion: FolderSuggestion) => {
      if (!sessionId) return;
      trackerRef.current?.accept(suggestion.name);
      setSuggestions((prev) => prev.filter((s) => s.id !== suggestion.id));
      await addSessionToFolder(sessionId, suggestion.id);
      trackerRef.current?.addExistingFolder(suggestion.id);

      const updatedHints = buildVocabularyHints(await listFolders(), await listTags());
      if (updatedHints) {
        commands.updateVocabularyHints(updatedHints).catch(() => {});
      }
    },
    [sessionId, addSessionToFolder],
  );

  const dismissSuggestion = useCallback((suggestion: FolderSuggestion) => {
    trackerRef.current?.dismiss(suggestion.name);
    setSuggestions((prev) => prev.filter((s) => s.id !== suggestion.id));
  }, []);

  return { suggestions, acceptSuggestion, dismissSuggestion };
}
