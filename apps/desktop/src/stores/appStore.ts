import { create } from "zustand";
import { persist } from "zustand/middleware";
import { commands } from "@/lib/tauri";
import type {
  CaptureStatusDto,
  BufferStatusDto,
  AudioDeviceInfoDto,
  ModelInfoDto,
  ModelSizeDto,
  CaptureSourceDto,
  MixConfigDto,
  LiveTranscriptionConfig,
  LiveTranscriptionPhase,
  LiveSegmentEvent,
  EngineKindDto,
  ParakeetVariantDto,
  EngineDescriptorDto,
  ParakeetModelInfoDto,
  SortformerModelInfoDto,
} from "@/lib/tauri";
import {
  createSession as dbCreateSession,
  updateSessionTitle,
  completeSession,
  deleteSession as dbDeleteSession,
  listSessions,
  getSession,
  getSessionSegments,
  insertSegment,
  deleteAllSessions,
  togglePin as dbTogglePin,
  createFolder as dbCreateFolder,
  updateFolder as dbUpdateFolder,
  updateSegmentText as dbUpdateSegmentText,
  softDeleteSegment as dbSoftDeleteSegment,
  toggleSegmentHidden as dbToggleSegmentHidden,
  createManualSession as dbCreateManualSession,
  deleteFolder as dbDeleteFolder,
  updateFolderParent as dbUpdateFolderParent,
  listFolders,
  updateSessionWavPath,
  addSessionToFolder as dbAddSessionToFolder,
  removeSessionFromFolder as dbRemoveSessionFromFolder,
  removeSessionFromAllFolders as dbRemoveSessionFromAllFolders,
  listAllSessionFolders,
  reorderFolders as dbReorderFolders,
  listDictationHistory,
  deleteDictationHistoryEntry as dbDeleteDictationHistoryEntry,
  getDictationHistoryEntry,
  clearDictationHistory as dbClearDictationHistory,
  clearDictationSessionLink,
} from "@/lib/db";
import type { DbDictationHistory } from "@/lib/db";
import { findBranchConflicts, buildFolderTree, buildChildMap, type FolderTreeNode } from "@/lib/folder-tree";
import type { DbSession, DbSegment, DbFolder } from "@/lib/db";
import { toast } from "sonner";
import { DEFAULT_AI_SETTINGS } from "@/lib/ai";
import type { AISettings } from "@/lib/ai";
import {
  trackSessionCreated,
  trackSessionStopped,
  trackSessionDeleted,
  trackSessionsCleared,
  trackManualNoteCreated,
  trackFolderCreated,
  trackSessionPinned,
  trackSessionUnpinned,
  trackModelDownloaded,
  trackModelDeleted,
  trackModelSwitched,
  trackEngineError,
  trackSettingChanged,
  trackSessionMovedToFolder,
} from "@/lib/analytics";

export type ThemeMode = "light" | "dark" | "system";

export type DictationOutputAction = "paste" | "clipboard" | "new-note";

export interface DictationSlot {
  id: string;
  name: string;
  enabled: boolean;
  aiEnabled: boolean;
  prompt: string;
  outputAction: DictationOutputAction;
  defaultBinding?: string;
}

export type DictationActivationMode = "hold" | "toggle";

export interface DictationSettings {
  enabled: boolean;
  activationMode: DictationActivationMode;
  slots: DictationSlot[];
}

export const DEFAULT_DICTATION_SLOTS: DictationSlot[] = [
  {
    id: "1",
    name: "Raw Dictation",
    enabled: true,
    aiEnabled: false,
    prompt: "",
    outputAction: "paste",
    defaultBinding: "Control+Shift+Space",
  },
];

export const DEFAULT_DICTATION_SETTINGS: DictationSettings = {
  enabled: true,
  activationMode: "hold",
  slots: DEFAULT_DICTATION_SLOTS,
};

export interface OnboardingState {
  completedFlows: Record<string, string>; // flowId → ISO timestamp
}

export interface Settings {
  captureSource: CaptureSourceDto;
  selectedMicDeviceId: string | null;
  selectedEngine: EngineKindDto;
  selectedModelSize: ModelSizeDto;
  selectedParakeetVariant: ParakeetVariantDto;
  diarizationEnabled: boolean;
  /// `{ [sessionId]: { [speakerId]: "Alice" } }`. Client-side persisted.
  /// Populated when Parakeet + Sortformer diarization tags segments; applies
  /// to the `SpeakerHeader` component. When diarization is off, segments
  /// fall back to source-based labels ("You" / "Other") via `SourceHeader`.
  speakerNames: Record<string, Record<number, string>>;
  language: string;
  mixConfig: MixConfigDto;
  bufferMaxSeconds: number;
  silenceDurationMs: number;
  maxChunkSeconds: number;
  promptContextChars: number;
  promptDecaySilenceSeconds: number;
  theme: ThemeMode;
  sidebarCollapsed: boolean;
  ai: AISettings;
  shortcutBindings: Record<string, string>;
  audioSaveLocation: string | null;
  audioExportFormat: "wav" | "mp3";
  mp3Bitrate: number;
  dictation: DictationSettings;
  showRecordingIndicator: boolean;
  onboarding: OnboardingState;
}

export type EnginePhase =
  | "idle"
  | "downloading"
  | "initializing"
  | "ready"
  | "error";

export interface ListFilter {
  type: "all" | "pinned" | "folder" | "dictation";
  folderId?: string;
}

interface AppState {
  // Engine setup
  enginePhase: EnginePhase;
  engineError: string | null;
  modelDownloadProgress: number | null;

  // Capture (pushed from backend events)
  captureStatus: CaptureStatusDto | null;
  bufferInfo: BufferStatusDto | null;

  // Navigation
  currentView: "note-list" | "note-detail" | "settings";
  selectedSessionId: string | null;
  listFilter: ListFilter;

  // Session list (from SQLite)
  sessions: DbSession[];

  // Folders
  folders: DbFolder[];
  folderTree: FolderTreeNode[];
  folderByIdMap: Map<string, DbFolder>;
  folderChildMap: Map<string, string[]>;
  sessionFolderMap: Record<string, string[]>;

  // Active recording session
  activeSessionId: string | null;
  activeSessionSegments: DbSegment[];
  activeSessionStartTime: number | null;

  // Viewing a completed session
  viewSessionSegments: DbSegment[];
  viewSession: DbSession | null;

  // Live transcription
  liveTranscriptionActive: boolean;
  livePhase: LiveTranscriptionPhase | null;
  backfillActive: boolean;
  // True from when the user clicks Stop until the backend's Stopped/Error
  // event fires. Lets the UI show a pulsing "Finalizing…" state immediately
  // while the engine drains the current chunk + finalizes the WAV.
  sessionStopping: boolean;

  // Note refresh (for cross-component refresh signaling)
  noteRefreshCounter: number;

  // Audio playback
  playbackTime: number;
  isPlaying: boolean;

  // Dictation history (runtime, loaded from SQLite)
  dictationHistory: DbDictationHistory[];

  // Update availability (runtime, not persisted)
  updateAvailable: { version: string; body: string | undefined } | null;
  updateDismissedVersion: string | null;

  // Settings (persisted)
  settings: Settings;

  // Devices (loaded once)
  devices: AudioDeviceInfoDto[];
  models: ModelInfoDto[];
  // Engine catalogue + Parakeet/Sortformer model state (loaded on autoSetup).
  // engineCatalogue is the static capability descriptor from the Rust side.
  // parakeetModels/sortformerStatus are the per-variant download status.
  engineCatalogue: EngineDescriptorDto[];
  parakeetModels: ParakeetModelInfoDto[];
  sortformerStatus: SortformerModelInfoDto | null;

  // Setters (called by event listeners)
  setCaptureStatus: (status: CaptureStatusDto) => void;
  setBufferInfo: (info: BufferStatusDto) => void;
  setModelDownloadProgress: (p: number | null) => void;

  // Live transcription setters
  setLivePhase: (phase: LiveTranscriptionPhase) => void;
  onLiveSegment: (event: LiveSegmentEvent) => void;
  onBackfillComplete: () => void;
  onSessionWavReady: (sessionId: string, filePath: string, durationSeconds: number) => void;
  recoverActiveSession: (sessionId: string, effectiveStartEpochMs?: number) => Promise<void>;

  // Actions
  autoSetup: () => Promise<void>;
  loadSessions: () => Promise<void>;
  loadFolders: () => Promise<void>;
  createAndStartSession: (backfillSeconds?: number, trigger?: string) => Promise<void>;
  stopActiveSession: () => Promise<void>;
  openSession: (id: string) => Promise<void>;
  deleteSession: (id: string) => Promise<void>;
  navigateTo: (
    view: "note-list" | "note-detail" | "settings",
    sessionId?: string,
  ) => void;
  setListFilter: (filter: ListFilter) => void;
  refreshDevices: () => Promise<void>;
  refreshModels: () => Promise<void>;
  updateSettings: (partial: Partial<Settings>) => void;
  clearAllSessions: () => Promise<void>;
  downloadModel: (size: ModelSizeDto) => Promise<void>;
  deleteModel: (size: ModelSizeDto) => Promise<void>;
  switchModel: (size: ModelSizeDto) => Promise<void>;

  // Engine + Parakeet/Sortformer actions
  loadEngineCatalogue: () => Promise<void>;
  refreshParakeetModels: () => Promise<void>;
  refreshSortformerStatus: () => Promise<void>;
  /// Switch the active transcription engine. Shuts down the current sidecar
  /// client and re-inits with the new engine. The frontend persists the
  /// selection via `updateSettings`; autoSetup picks it up on next start.
  switchEngine: (engine: EngineKindDto) => Promise<void>;
  downloadParakeetModel: (variant: ParakeetVariantDto) => Promise<void>;
  deleteParakeetModel: (variant: ParakeetVariantDto) => Promise<void>;
  /// Toggle speaker diarization. When flipped on for the first time and
  /// Sortformer is missing, downloads it. Re-inits the client so the new
  /// flag takes effect on the next transcribe call.
  setDiarizationEnabled: (enabled: boolean) => Promise<void>;
  /// Rename a speaker for a single session. Pass an empty/whitespace `name`
  /// to clear the override and fall back to the default `Speaker N` label.
  setSpeakerName: (sessionId: string, speakerId: number, name: string) => void;

  // Folder actions
  createFolder: (name: string, parentId?: string | null, icon?: string | null, color?: string | null, description?: string | null) => Promise<void>;
  updateFolder: (id: string, updates: { name?: string; icon?: string | null; color?: string | null; description?: string | null }) => Promise<void>;
  deleteFolder: (id: string) => Promise<void>;
  moveFolder: (folderId: string, newParentId: string | null) => Promise<void>;
  reorderFolders: (folderId: string, overFolderId: string) => Promise<void>;
  loadSessionFolders: () => Promise<void>;

  // Pin/folder session actions
  togglePin: (sessionId: string) => Promise<void>;
  toggleSessionFolder: (sessionId: string, folderId: string) => Promise<void>;
  addSessionToFolder: (sessionId: string, folderId: string) => Promise<void>;
  removeSessionFromAllFolders: (sessionId: string) => Promise<void>;

  // Segment editing actions
  editSegmentText: (segmentId: string, newText: string) => Promise<void>;
  deleteSegment: (segmentId: string) => Promise<void>;
  toggleSegmentHidden: (segmentId: string) => Promise<void>;
  showHiddenSegments: boolean;
  setShowHiddenSegments: (show: boolean) => void;
  refreshViewSessionSegments: () => Promise<void>;

  // Sidebar
  toggleSidebar: () => void;

  // Manual notes
  createManualNote: (title?: string) => Promise<void>;

  // Note refresh
  incrementNoteRefresh: () => void;

  // Audio playback
  setPlaybackTime: (time: number) => void;
  setIsPlaying: (playing: boolean) => void;

  // Dictation history
  loadDictationHistory: () => Promise<void>;
  deleteDictationHistoryEntry: (id: string) => Promise<void>;
  clearDictationHistory: () => Promise<void>;

  // Onboarding
  completeFlow: (flowId: string) => void;

  // Update
  setUpdateAvailable: (update: { version: string; body: string | undefined } | null) => void;
  dismissUpdate: () => void;
}

/**
 * Serialization queue for onLiveSegment.
 * Concurrent backfill + live events can interleave async reads/writes to
 * activeSessionSegments. This queue ensures only one handler runs at a time.
 */
let segmentQueueTail: Promise<void> = Promise.resolve();
let lastSessionsRefreshTime = 0;
function enqueueSegmentWork(fn: () => Promise<void>): void {
  segmentQueueTail = segmentQueueTail.then(fn, fn);
}

const defaultSettings: Settings = {
  captureSource: "Mixed",
  selectedMicDeviceId: null,
  // Whisper stays the default for fresh installs and for upgrade users
  // (the migration adds this field without changing the active engine).
  selectedEngine: "Whisper",
  selectedModelSize: "Small",
  selectedParakeetVariant: "TdtV3",
  diarizationEnabled: false,
  speakerNames: {},
  language: "en",
  mixConfig: { mic_gain: 1.0, system_gain: 1.0, normalize: false },
  bufferMaxSeconds: 300,
  silenceDurationMs: 800,
  maxChunkSeconds: 30,
  promptContextChars: 350,
  promptDecaySilenceSeconds: 5,
  theme: "system",
  sidebarCollapsed: false,
  ai: DEFAULT_AI_SETTINGS,
  shortcutBindings: {},
  audioSaveLocation: null,
  audioExportFormat: "mp3",
  mp3Bitrate: 64,
  dictation: DEFAULT_DICTATION_SETTINGS,
  showRecordingIndicator: true,
  onboarding: { completedFlows: {} },
};

function updateSessionFolderMap(
  current: Record<string, string[]>,
  sessionId: string,
  add: string[],
  remove: string[],
): Record<string, string[]> {
  const ids = (current[sessionId] ?? []).filter((id) => !remove.includes(id));
  for (const id of add) {
    if (!ids.includes(id)) ids.push(id);
  }
  if (ids.length === 0) {
    const { [sessionId]: _, ...rest } = current;
    return rest;
  }
  return { ...current, [sessionId]: ids };
}

async function addSessionToFolderWithConflicts(
  sessionId: string,
  folderId: string,
  currentFolderIds: string[],
  folders: DbFolder[],
): Promise<string[]> {
  const conflicts = findBranchConflicts(folders, currentFolderIds, folderId);
  for (const cId of conflicts) {
    await dbRemoveSessionFromFolder(sessionId, cId);
  }
  await dbAddSessionToFolder(sessionId, folderId);
  return conflicts;
}

function deriveFolderState(folders: DbFolder[]) {
  return {
    folderTree: buildFolderTree(folders),
    folderByIdMap: new Map(folders.map((f) => [f.id, f])),
    folderChildMap: buildChildMap(folders),
  };
}

function createAppStore() {
  return create<AppState>()(
    persist(
      (set, get) => ({
      // Initial state
      enginePhase: "idle",
      engineError: null,
      modelDownloadProgress: null,
      captureStatus: null,
      bufferInfo: null,
      currentView: "note-list",
      selectedSessionId: null,
      listFilter: { type: "all" },
      sessions: [],
      folders: [],
      folderTree: [],
      folderByIdMap: new Map(),
      folderChildMap: new Map(),
      sessionFolderMap: {},
      activeSessionId: null,
      activeSessionSegments: [],
      activeSessionStartTime: null,
      viewSessionSegments: [],
      viewSession: null,
      liveTranscriptionActive: false,
      livePhase: null,
      backfillActive: false,
      sessionStopping: false,
      noteRefreshCounter: 0,
      playbackTime: 0,
      isPlaying: false,
      dictationHistory: [],
      updateAvailable: null,
      updateDismissedVersion: null,
      settings: defaultSettings,
      devices: [],
      models: [],
      engineCatalogue: [],
      parakeetModels: [],
      sortformerStatus: null,
      showHiddenSegments: false,

      // Setters
      setCaptureStatus: (status) => set({ captureStatus: status }),
      setBufferInfo: (info) => set({ bufferInfo: info }),
      setModelDownloadProgress: (p) => set({ modelDownloadProgress: p }),
      setLivePhase: (phase) => {
        const active = phase === "Running";
        set({ livePhase: phase, liveTranscriptionActive: active });

        // When backend reports Stopped or Error, finalize the session.
        // Enqueue finalization on segmentQueueTail so pending segment writes
        // complete before activeSessionId is cleared (prevents race condition).
        if (phase === "Stopped" || phase === "Error") {
          const { activeSessionId, activeSessionStartTime, activeSessionSegments } = get();
          if (activeSessionId) {
            const capturedSessionId = activeSessionId;
            const durationSeconds = activeSessionStartTime
              ? (Date.now() - activeSessionStartTime) / 1000
              : 0;

            enqueueSegmentWork(async () => {
              trackSessionStopped({
                duration_seconds: Math.round(durationSeconds),
                segment_count: activeSessionSegments.length,
              });
              await completeSession(capturedSessionId, durationSeconds).catch((e) => {
                console.error("Failed to complete session:", e);
              });

              // WAV is streamed during the session and finalized by the backend.
              // The "session-wav-ready" event (handled in useLiveTranscriptionEvents)
              // updates the DB with the file path and duration.

              set({
                activeSessionId: null,
                activeSessionStartTime: null,
                liveTranscriptionActive: false,
                livePhase: null,
                backfillActive: false,
                sessionStopping: false,
              });

              // Reload session data — re-read selectedSessionId inside .then()
              // to guard against the user navigating away before resolution.
              try {
                const sessions = await listSessions();
                const { selectedSessionId } = get();
                if (selectedSessionId === capturedSessionId) {
                  const [session, segments] = await Promise.all([
                    getSession(capturedSessionId),
                    getSessionSegments(capturedSessionId),
                  ]);
                  // Re-check: user may have navigated during the inner await
                  if (get().selectedSessionId === capturedSessionId) {
                    set({
                      sessions,
                      viewSession: session,
                      viewSessionSegments: segments,
                    });
                  } else {
                    set({ sessions });
                  }
                } else {
                  set({ sessions });
                }
              } catch (e) {
                console.error("Failed to reload sessions:", e);
              }
            });
          }
        }
      },

      onLiveSegment: (event: LiveSegmentEvent) => {
        enqueueSegmentWork(async () => {
          console.debug(
            `[segment] source=${event.source} chunk=${event.chunk_index} ` +
              `offset=${event.audio_offset_seconds.toFixed(2)}s ` +
              `duration=${event.chunk_duration_seconds.toFixed(2)}s ` +
              `segments=${event.segments.length} ` +
              `backfill=${event.is_backfill}`,
          );

          // Prefer the session_id carried on the event so late-arriving
          // segments (after `setLivePhase("Stopped")` has cleared
          // activeSessionId) still persist to the right session in the DB.
          // Fall back to ambient activeSessionId for backward compat with any
          // older event payload that didn't carry session_id.
          const { activeSessionId } = get();
          const targetSessionId = event.session_id ?? activeSessionId;
          if (!targetSessionId) return;

          // Create one segment per Whisper/Parakeet segment to preserve timestamps
          const newSegments: DbSegment[] = [];
          for (const seg of event.segments) {
            const text = seg.text.trim();
            if (!text) continue;
            newSegments.push({
              id: crypto.randomUUID(),
              session_id: targetSessionId,
              source: event.source,
              text,
              audio_offset_seconds:
                event.audio_offset_seconds + seg.start_ms / 1000,
              chunk_duration_seconds: (seg.end_ms - seg.start_ms) / 1000,
              confidence: seg.confidence,
              created_at: new Date().toISOString(),
              chunk_index: event.chunk_index,
              original_text: null,
              edited_at: null,
              deleted_at: null,
              hidden: 0,
              speaker_id: seg.speaker_id ?? null,
            });
          }

          if (newSegments.length === 0) return;

          try {
            for (const segment of newSegments) {
              await insertSegment(segment);
            }

            // DB persistence is done. The remaining work updates the in-memory
            // active-session view; only do it if the target session is still
            // the active one. Late arrivals after stop persist to DB but don't
            // disturb whatever the user navigated to.
            const currentActiveId = get().activeSessionId;
            if (!currentActiveId || currentActiveId !== targetSessionId) return;

            // Auto-title from first segment — re-read state to avoid race
            const currentSegments = get().activeSessionSegments;
            if (currentSegments.length === 0) {
              const title = newSegments[0].text.slice(0, 60);
              await updateSessionTitle(targetSessionId, title);
            }

            // Re-check after the title update await
            if (get().activeSessionId !== targetSessionId) return;

            // Re-read activeSessionSegments after awaits to avoid overwriting
            // segments inserted by a concurrent onLiveSegment call
            const latestSegments = get().activeSessionSegments;
            const updated = [...latestSegments, ...newSegments].sort(
              (a, b) => a.audio_offset_seconds - b.audio_offset_seconds,
            );
            set({ activeSessionSegments: updated });

            // Refresh only the active session in the sidebar (avoids full SELECT *)
            // Throttled to max 1x/sec — sidebar lag is imperceptible,
            // and setLivePhase("Stopped") does a full loadSessions() for final state.
            const now = Date.now();
            if (now - lastSessionsRefreshTime >= 1000) {
              lastSessionsRefreshTime = now;
              const freshSession = await getSession(targetSessionId);
              if (freshSession) {
                set({
                  sessions: get().sessions.map((s) =>
                    s.id === targetSessionId ? freshSession : s,
                  ),
                });
              }
            }
          } catch (e) {
            console.error("Failed to persist live segment:", e);
            toast.error("Failed to save transcript segment", { id: "segment-write-error" });
          }
        });
      },

      onBackfillComplete: () => {
        set({ backfillActive: false });
      },

      onSessionWavReady: (sessionId, filePath, durationSeconds) => {
        updateSessionWavPath(sessionId, filePath, durationSeconds)
          .then(() => {
            const { selectedSessionId } = get();
            if (selectedSessionId === sessionId) {
              getSession(sessionId).then((session) => {
                if (session && get().selectedSessionId === sessionId) {
                  set({ viewSession: session });
                }
              });
            }
          })
          .catch((e) => {
            console.error("Failed to update session WAV path:", e);
          });
      },

      recoverActiveSession: async (sessionId: string, effectiveStartEpochMs?: number) => {
        // If we already have this session active, skip
        if (get().activeSessionId === sessionId) return;

        try {
          const segments = await getSessionSegments(sessionId);
          const session = await getSession(sessionId);

          set({
            activeSessionId: sessionId,
            activeSessionSegments: segments,
            liveTranscriptionActive: true,
            activeSessionStartTime: effectiveStartEpochMs
              ?? (session ? new Date(session.created_at + "Z").getTime() : Date.now()),
          });

          if (session) {
            set({
              selectedSessionId: sessionId,
              viewSession: session,
              viewSessionSegments: segments,
              currentView: "note-detail",
            });
          }
        } catch (e) {
          console.error("Failed to recover active session:", e);
        }
      },

      // Actions
      loadSessions: async () => {
        try {
          const sessions = await listSessions();
          set({ sessions });
        } catch (e) {
          console.error("Failed to load sessions:", e);
        }
      },

      loadFolders: async () => {
        try {
          const folders = await listFolders();
          set({ folders, ...deriveFolderState(folders) });
        } catch (e) {
          console.error("Failed to load folders:", e);
        }
      },

      loadSessionFolders: async () => {
        try {
          const rows = await listAllSessionFolders();
          const map: Record<string, string[]> = {};
          for (const row of rows) {
            if (!map[row.session_id]) map[row.session_id] = [];
            map[row.session_id].push(row.folder_id);
          }
          set({ sessionFolderMap: map });
        } catch (e) {
          console.error("Failed to load session folders:", e);
        }
      },

      createAndStartSession: async (backfillSeconds?: number, trigger?: string) => {
        const { settings, enginePhase, captureStatus } = get();

        if (enginePhase !== "ready") {
          throw new Error("Engine is not ready");
        }
        if (captureStatus?.state !== "Capturing") {
          throw new Error("Audio capture is not active");
        }

        const sessionId = crypto.randomUUID();

        await dbCreateSession(sessionId, settings.captureSource);

        const config: LiveTranscriptionConfig = {
          silence_threshold: 0.01,
          silence_duration_ms: settings.silenceDurationMs,
          max_chunk_seconds: settings.maxChunkSeconds,
          backfill_seconds: backfillSeconds ?? 0,
          source: settings.captureSource,
          mix_config:
            settings.captureSource === "Mixed" ? settings.mixConfig : null,
          language: settings.language,
          prompt_context_chars: settings.promptContextChars,
          prompt_decay_silence_seconds:
            settings.promptDecaySilenceSeconds > 0
              ? settings.promptDecaySilenceSeconds
              : null,
          session_id: sessionId,
          audio_save_location: settings.audioSaveLocation,
          audio_export_format: settings.audioExportFormat,
          mp3_bitrate: settings.audioExportFormat === "mp3" ? settings.mp3Bitrate : null,
          diarization:
            settings.selectedEngine === "Parakeet" && settings.diarizationEnabled,
        };

        const result = await commands.startLiveTranscription(config);
        if (result.status === "error") {
          // Clean up the DB row we just created
          await dbDeleteSession(sessionId).catch(() => {});
          throw new Error(result.error.message);
        }

        trackSessionCreated({
          source: settings.captureSource,
          backfill_seconds: backfillSeconds ?? 0,
          trigger: trigger ?? "unknown",
        });

        set({
          activeSessionId: sessionId,
          activeSessionSegments: [],
          activeSessionStartTime: result.data.effective_start_epoch_ms,
          liveTranscriptionActive: true,
          livePhase: "Running",
          currentView: "note-detail",
          selectedSessionId: sessionId,
          backfillActive: (backfillSeconds ?? 0) > 0,
        });

        // Reload sidebar
        const sessions = await listSessions();
        set({ sessions });
      },

      stopActiveSession: async () => {
        const { activeSessionId, sessionStopping } = get();
        if (!activeSessionId || sessionStopping) return;

        // Flip the UI immediately — the Rust command returns right away but
        // the loop still needs to drain the current chunk and finalize the
        // WAV before emitting Stopped. Users shouldn't see the stop button
        // appear to hang during that window.
        set({ sessionStopping: true });

        try {
          await commands.stopLiveTranscription();
        } catch (e) {
          console.error("Failed to stop session:", e);
          set({ sessionStopping: false });
        }
      },

      openSession: async (id: string) => {
        const { activeSessionId } = get();

        if (id === activeSessionId) {
          set({
            currentView: "note-detail",
            selectedSessionId: id,
          });
          return;
        }

        try {
          const session = await getSession(id);
          const segments = await getSessionSegments(id);
          set({
            currentView: "note-detail",
            selectedSessionId: id,
            viewSession: session,
            viewSessionSegments: segments,
          });
        } catch (e) {
          console.error("Failed to open session:", e);
          toast.error("Failed to open session");
        }
      },

      deleteSession: async (id: string) => {
        const { activeSessionId, selectedSessionId } = get();

        // Can't delete active recording session
        if (id === activeSessionId) return;

        try {
          // Delete WAV file if it exists
          commands.deleteSessionWav(id, get().settings.audioSaveLocation).catch(() => {});

          await dbDeleteSession(id);
          await clearDictationSessionLink(id);
          trackSessionDeleted();
          const sessions = await listSessions();
          const { [id]: _, ...restMap } = get().sessionFolderMap;

          // Refresh dictation history if viewing dictation list
          if (get().listFilter.type === "dictation") {
            get().loadDictationHistory();
          }

          if (selectedSessionId === id) {
            set({
              sessions,
              sessionFolderMap: restMap,
              currentView: "note-list",
              selectedSessionId: null,
              viewSession: null,
              viewSessionSegments: [],
            });
          } else {
            set({ sessions, sessionFolderMap: restMap });
          }
        } catch (e) {
          console.error("Failed to delete session:", e);
          toast.error("Failed to delete session");
        }
      },

      navigateTo: (view, sessionId) => {
        set({
          currentView: view,
          selectedSessionId: sessionId ?? null,
        });
      },

      setListFilter: (filter) => {
        set({ listFilter: filter });
      },

      refreshDevices: async () => {
        try {
          const result = await commands.listAudioDevices();
          if (result.status === "ok") {
            set({ devices: result.data });
          }
        } catch (e) {
          console.error("Failed to refresh devices:", e);
        }
      },

      refreshModels: async () => {
        try {
          const result = await commands.getAvailableModels();
          if (result.status === "ok") {
            set({ models: result.data });
          }
        } catch (e) {
          console.error("Failed to refresh models:", e);
        }
      },

      autoSetup: async () => {
        // HMR-preserved Zustand store already holds "ready" when the backend
        // is warm — skip the whole setup path. On fresh app start enginePhase
        // is "idle", so this falls through to full init. Backend commands are
        // idempotent (startCapture treats AlreadyRunning as Ok,
        // init_transcription_client short-circuits on is_some()), so any
        // duplicate calls are harmless.
        if (get().enginePhase === "ready") return;

        const { settings } = get();

        // Request Screen Recording TCC up front — on macOS 14.2+ this gates
        // the Core Audio tap cpal uses for system audio loopback. Fire the
        // prompt before startCapture so users see it during onboarding instead
        // of later, silently failing.
        commands.requestScreenCapturePermission().catch(() => {});

        // Track 1 — Start capture (fire and forget, errors surface via capture-status events)
        commands
          .startCapture(
            settings.selectedMicDeviceId,
            settings.captureSource,
            settings.bufferMaxSeconds,
          )
          .then((result) => {
            if (result.status === "error") {
              commands.getCaptureStatus().then((r) => {
                if (r.status === "ok") get().setCaptureStatus(r.data);
              });
            }
          })
          .catch(() => {});

        // Track 2 — Engine setup
        try {
          // Load devices + catalogue in parallel (non-blocking)
          get()
            .refreshDevices()
            .catch((e) => console.error("Failed to refresh devices:", e));
          get()
            .loadEngineCatalogue()
            .catch((e) => console.error("Failed to load catalogue:", e));

          if (settings.selectedEngine === "Parakeet") {
            // Parakeet path
            await get().refreshParakeetModels();
            await get().refreshSortformerStatus();
            const variant = settings.selectedParakeetVariant;
            const ready = get().parakeetModels.find(
              (m) => m.variant === variant && m.downloaded,
            );
            if (!ready) {
              set({ enginePhase: "downloading", modelDownloadProgress: 0 });
              const dl = await commands.downloadParakeetModel(variant);
              if (dl.status === "error") {
                trackEngineError({ error: dl.error.message, phase: "downloading" });
                set({
                  enginePhase: "error",
                  engineError: dl.error.message,
                  modelDownloadProgress: null,
                });
                return;
              }
              set({ modelDownloadProgress: null });
              await get().refreshParakeetModels();
            }

            // Sortformer download is lazy — only when diarization is on.
            if (
              settings.diarizationEnabled &&
              !get().sortformerStatus?.downloaded
            ) {
              set({ modelDownloadProgress: 0 });
              const dl = await commands.downloadSortformerModel("V2_1");
              if (dl.status === "error") {
                console.error(
                  "sortformer download failed; continuing without diarization:",
                  dl.error.message,
                );
              }
              set({ modelDownloadProgress: null });
              await get().refreshSortformerStatus();
            }

            set({ enginePhase: "initializing" });
            const initResult = await commands.initTranscriptionClient(
              "Parakeet",
              null,
              variant,
              settings.diarizationEnabled,
            );
            if (initResult.status === "error") {
              trackEngineError({ error: initResult.error.message, phase: "initializing" });
              set({ enginePhase: "error", engineError: initResult.error.message });
              return;
            }
            set({ enginePhase: "ready", engineError: null });
            return;
          }

          // Whisper path (default).
          const modelsResult = await commands.getAvailableModels();
          if (modelsResult.status === "ok") {
            set({ models: modelsResult.data });
          }

          const models = get().models;
          const selectedModel = models.find(
            (m) => m.size === settings.selectedModelSize,
          );

          if (!selectedModel?.downloaded) {
            set({ enginePhase: "downloading", modelDownloadProgress: 0 });
            const downloadResult = await commands.downloadModel(
              settings.selectedModelSize,
            );
            if (downloadResult.status === "error") {
              trackEngineError({ error: downloadResult.error.message, phase: "downloading" });
              set({
                enginePhase: "error",
                engineError: downloadResult.error.message,
                modelDownloadProgress: null,
              });
              return;
            }
            set({ modelDownloadProgress: null });
            await get().refreshModels();
          }

          set({ enginePhase: "initializing" });
          const initResult = await commands.initTranscriptionClient(
            "Whisper",
            settings.selectedModelSize,
            null,
            false,
          );
          if (initResult.status === "error") {
            trackEngineError({ error: initResult.error.message, phase: "initializing" });
            set({ enginePhase: "error", engineError: initResult.error.message });
            return;
          }

          set({ enginePhase: "ready", engineError: null });
        } catch (e) {
          set({ enginePhase: "error", engineError: String(e) });
        }
      },

      downloadModel: async (size: ModelSizeDto) => {
        set({ modelDownloadProgress: 0 });
        try {
          const result = await commands.downloadModel(size);
          if (result.status === "error") {
            throw new Error(result.error.message);
          }
          trackModelDownloaded({ model_size: size });
          await get().refreshModels();
        } finally {
          set({ modelDownloadProgress: null });
        }
      },

      deleteModel: async (size: ModelSizeDto) => {
        const result = await commands.deleteModel(size);
        if (result.status === "error") {
          throw new Error(result.error.message);
        }
        trackModelDeleted({ model_size: size });
        await get().refreshModels();
      },

      switchModel: async (size: ModelSizeDto) => {
        const { models, settings: { selectedModelSize: fromSize } } = get();
        const model = models.find((m) => m.size === size);

        try {
          // Download if needed
          if (!model?.downloaded) {
            set({ enginePhase: "downloading", modelDownloadProgress: 0 });
            const downloadResult = await commands.downloadModel(size);
            if (downloadResult.status === "error") {
              throw new Error(downloadResult.error.message);
            }
            set({ modelDownloadProgress: null });
            await get().refreshModels();
          }

          // Shutdown current engine
          set({ enginePhase: "initializing" });
          await commands.shutdownTranscriptionClient();

          // Init with new model
          const initResult = await commands.initTranscriptionClient(
            "Whisper",
            size,
            null,
            false,
          );
          if (initResult.status === "error") {
            throw new Error(initResult.error.message);
          }

          trackModelSwitched({ model_size: size, from_size: fromSize });
          set({
            enginePhase: "ready",
            engineError: null,
          });
          get().updateSettings({ selectedModelSize: size });
        } catch (e) {
          set({ enginePhase: "error", engineError: String(e) });
        }
      },

      // ---------- Engine + Parakeet/Sortformer actions ----------

      loadEngineCatalogue: async () => {
        try {
          const r = await commands.getEngineCatalogue();
          if (r.status === "ok") set({ engineCatalogue: r.data });
        } catch (e) {
          console.error("Failed to load engine catalogue:", e);
        }
      },

      refreshParakeetModels: async () => {
        try {
          const r = await commands.getParakeetModels();
          if (r.status === "ok") set({ parakeetModels: r.data });
        } catch (e) {
          console.error("Failed to refresh parakeet models:", e);
        }
      },

      refreshSortformerStatus: async () => {
        try {
          const r = await commands.getSortformerStatus();
          if (r.status === "ok") set({ sortformerStatus: r.data });
        } catch (e) {
          console.error("Failed to refresh sortformer status:", e);
        }
      },

      switchEngine: async (engine: EngineKindDto) => {
        const settings = get().settings;
        if (settings.selectedEngine === engine) return;
        const previousEngine = settings.selectedEngine;

        // Flip the UI to the target engine up front so any download shows
        // progress in the same model row the user would see after the switch
        // completes — no top-of-screen banner, no cross-engine flicker.
        get().updateSettings({ selectedEngine: engine });

        try {
          if (engine === "Parakeet") {
            await get().refreshParakeetModels();
            const variant = settings.selectedParakeetVariant;
            const ready = get().parakeetModels.find(
              (m) => m.variant === variant && m.downloaded,
            );
            if (!ready) {
              set({ modelDownloadProgress: 0 });
              const dl = await commands.downloadParakeetModel(variant);
              if (dl.status === "error") {
                set({ modelDownloadProgress: null });
                throw new Error(dl.error.message);
              }
              set({ modelDownloadProgress: null });
              await get().refreshParakeetModels();
            }
            if (settings.diarizationEnabled) {
              await get().refreshSortformerStatus();
              if (!get().sortformerStatus?.downloaded) {
                set({ modelDownloadProgress: 0 });
                const dl = await commands.downloadSortformerModel("V2_1");
                if (dl.status === "error") {
                  console.error(
                    "sortformer download failed; continuing without diarization:",
                    dl.error.message,
                  );
                }
                set({ modelDownloadProgress: null });
                await get().refreshSortformerStatus();
              }
            }
          } else if (engine === "Whisper") {
            await get().refreshModels();
            const ready = get().models.find(
              (m) => m.size === settings.selectedModelSize && m.downloaded,
            );
            if (!ready) {
              set({ modelDownloadProgress: 0 });
              const dl = await commands.downloadModel(
                settings.selectedModelSize,
              );
              if (dl.status === "error") {
                set({ modelDownloadProgress: null });
                throw new Error(dl.error.message);
              }
              set({ modelDownloadProgress: null });
              await get().refreshModels();
            }
          }

          set({ enginePhase: "initializing" });
          await commands.shutdownTranscriptionClient();

          const initResult = await commands.initTranscriptionClient(
            engine,
            engine === "Whisper" ? settings.selectedModelSize : null,
            engine === "Parakeet" ? settings.selectedParakeetVariant : null,
            engine === "Parakeet" ? settings.diarizationEnabled : false,
          );
          if (initResult.status === "error") {
            throw new Error(initResult.error.message);
          }

          set({ enginePhase: "ready", engineError: null });
        } catch (e) {
          get().updateSettings({ selectedEngine: previousEngine });
          set({ enginePhase: "error", engineError: String(e) });
          throw e;
        }
      },

      downloadParakeetModel: async (variant: ParakeetVariantDto) => {
        set({ modelDownloadProgress: 0 });
        try {
          const result = await commands.downloadParakeetModel(variant);
          if (result.status === "error") {
            throw new Error(result.error.message);
          }
          await get().refreshParakeetModels();
        } finally {
          set({ modelDownloadProgress: null });
        }
      },

      setSpeakerName: (sessionId: string, speakerId: number, name: string) => {
        const trimmed = name.trim();
        const current = get().settings.speakerNames[sessionId]?.[speakerId];
        if ((current ?? "") === trimmed) return;
        set((state) => {
          const next = { ...state.settings.speakerNames };
          const sessionMap = { ...(next[sessionId] ?? {}) };
          if (trimmed === "") {
            delete sessionMap[speakerId];
          } else {
            sessionMap[speakerId] = trimmed;
          }
          if (Object.keys(sessionMap).length === 0) {
            delete next[sessionId];
          } else {
            next[sessionId] = sessionMap;
          }
          return {
            settings: { ...state.settings, speakerNames: next },
          };
        });
      },


      deleteParakeetModel: async (variant: ParakeetVariantDto) => {
        const result = await commands.deleteParakeetModel(variant);
        if (result.status === "error") {
          throw new Error(result.error.message);
        }
        await get().refreshParakeetModels();
      },

      setDiarizationEnabled: async (enabled: boolean) => {
        // Diarization is intentionally locked off pending session-stable
        // speaker IDs. Sortformer::diarize() resets state per call, so the
        // speaker_id values it returns are chunk-local — the same person
        // can flip between Speaker 0 / Speaker 1 across chunk boundaries,
        // and the transcript UI groups + renames by that numeric id. The
        // entire IPC + sidecar + DB plumbing stays in place so re-enabling
        // is a one-line change once the chunk-local issue is resolved
        // (streaming Sortformer state, post-session pass on the full WAV,
        // or an embedding-based session registry — see the doc comment on
        // ParakeetBackend::run_diarization).
        //
        // Force enabled = false. We never persist `true` from this path.
        // Any caller that explicitly tries to enable gets a clear error.
        if (enabled) {
          throw new Error(
            "Speaker diarization is currently disabled — chunk-local speaker " +
              "IDs cause unstable labels across the session. Re-enable will " +
              "land once session-stable IDs are wired up.",
          );
        }
        const settings = get().settings;
        if (!settings.diarizationEnabled) return;

        get().updateSettings({ diarizationEnabled: false });

        if (settings.selectedEngine === "Parakeet") {
          set({ enginePhase: "initializing" });
          await commands.shutdownTranscriptionClient();
          const r = await commands.initTranscriptionClient(
            "Parakeet",
            null,
            settings.selectedParakeetVariant,
            false,
          );
          if (r.status === "error") {
            set({ enginePhase: "error", engineError: r.error.message });
            throw new Error(r.error.message);
          }
          set({ enginePhase: "ready", engineError: null });
        }
      },

      updateSettings: (partial) => {
        set((state) => ({
          settings: { ...state.settings, ...partial },
        }));

        const trackedKeys = [
          "captureSource", "theme", "language", "silenceDurationMs",
          "maxChunkSeconds", "promptContextChars",
          "promptDecaySilenceSeconds", "bufferMaxSeconds", "showRecordingIndicator",
        ] as const;
        for (const key of trackedKeys) {
          if (key in partial) {
            trackSettingChanged({ setting_name: key, new_value: String(partial[key]) });
          }
        }

        const needsRestart =
          partial.captureSource !== undefined ||
          partial.selectedMicDeviceId !== undefined ||
          partial.bufferMaxSeconds !== undefined;

        // Don't restart capture during an active live transcription session —
        // it would corrupt the in-progress recording.
        if (
          needsRestart &&
          get().captureStatus?.state === "Capturing" &&
          get().activeSessionId === null
        ) {
          const next = get().settings;
          commands
            .stopCapture()
            .then(() =>
              commands.startCapture(
                next.selectedMicDeviceId,
                next.captureSource,
                next.bufferMaxSeconds,
              ),
            )
            .catch((e) => {
              console.error("Failed to restart capture after settings change:", e);
              toast.error("Failed to restart capture");
            });
        }
      },

      clearAllSessions: async () => {
        if (get().activeSessionId) {
          toast.error("Cannot clear sessions while recording is active");
          return;
        }
        try {
          // Clean up WAV files for all sessions (fire-and-forget)
          const audioSaveLocation = get().settings.audioSaveLocation;
          for (const session of get().sessions) {
            commands.deleteSessionWav(session.id, audioSaveLocation).catch(() => {});
          }
          await deleteAllSessions();
          trackSessionsCleared();
          set({
            sessions: [],
            sessionFolderMap: {},
            currentView: "note-list",
            selectedSessionId: null,
            viewSession: null,
            viewSessionSegments: [],
          });
        } catch (e) {
          console.error("Failed to clear all sessions:", e);
          toast.error("Failed to clear sessions");
        }
      },

      // Folder actions
      createFolder: async (name: string, parentId?: string | null, icon?: string | null, color?: string | null, description?: string | null) => {
        try {
          const id = crypto.randomUUID();
          await dbCreateFolder(id, name, parentId ?? null, icon ?? null, color ?? null, description ?? null);
          trackFolderCreated();
          const folders = await listFolders();
          set({ folders, ...deriveFolderState(folders) });
        } catch (e) {
          console.error("Failed to create folder:", e);
          toast.error("Failed to create folder");
        }
      },

      updateFolder: async (id: string, updates: { name?: string; icon?: string | null; color?: string | null; description?: string | null }) => {
        try {
          await dbUpdateFolder(id, updates);
          const folders = await listFolders();
          set({ folders, ...deriveFolderState(folders) });
        } catch (e) {
          console.error("Failed to update folder:", e);
          toast.error("Failed to update folder");
        }
      },

      deleteFolder: async (id: string) => {
        try {
          await dbDeleteFolder(id);
          const folders = await listFolders();
          const { listFilter } = get();
          const newFilter =
            listFilter.type === "folder" && listFilter.folderId === id
              ? { type: "all" as const }
              : listFilter;
          set({ folders, listFilter: newFilter, ...deriveFolderState(folders) });
          await get().loadSessionFolders();
        } catch (e) {
          console.error("Failed to delete folder:", e);
          toast.error("Failed to delete folder");
        }
      },

      moveFolder: async (folderId: string, newParentId: string | null) => {
        try {
          await dbUpdateFolderParent(folderId, newParentId);
          const folders = await listFolders();
          set({ folders, ...deriveFolderState(folders) });
        } catch (e) {
          console.error("Failed to move folder:", e);
          toast.error("Failed to move folder");
        }
      },

      reorderFolders: async (folderId: string, overFolderId: string) => {
        try {
          const { folders } = get();
          const dragged = folders.find((f) => f.id === folderId);
          const over = folders.find((f) => f.id === overFolderId);
          if (!dragged || !over || dragged.parent_id !== over.parent_id) return;

          // Get siblings sorted by current sort_order then name
          const siblings = folders
            .filter((f) => f.parent_id === dragged.parent_id)
            .sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name));

          const oldIndex = siblings.findIndex((f) => f.id === folderId);
          const newIndex = siblings.findIndex((f) => f.id === overFolderId);
          if (oldIndex === -1 || newIndex === -1 || oldIndex === newIndex) return;

          // Reorder: remove from old position and insert at new
          const reordered = [...siblings];
          const [moved] = reordered.splice(oldIndex, 1);
          reordered.splice(newIndex, 0, moved);

          const updates = reordered.map((f, i) => ({ id: f.id, sort_order: i }));
          await dbReorderFolders(updates);
          const freshFolders = await listFolders();
          set({ folders: freshFolders, ...deriveFolderState(freshFolders) });
        } catch (e) {
          console.error("Failed to reorder folders:", e);
          toast.error("Failed to reorder folders");
        }
      },

      togglePin: async (sessionId: string) => {
        try {
          const wasPinned = get().sessions.find((s) => s.id === sessionId)?.is_pinned;
          await dbTogglePin(sessionId);
          if (wasPinned) trackSessionUnpinned();
          else trackSessionPinned();
          const sessions = await listSessions();
          set({ sessions });
        } catch (e) {
          console.error("Failed to toggle pin:", e);
          toast.error("Failed to toggle pin");
        }
      },

      toggleSessionFolder: async (sessionId: string, folderId: string) => {
        try {
          const { sessionFolderMap, folders } = get();
          const current = sessionFolderMap[sessionId] ?? [];
          const isRemoving = current.includes(folderId);
          if (isRemoving) {
            await dbRemoveSessionFromFolder(sessionId, folderId);
            set({ sessionFolderMap: updateSessionFolderMap(get().sessionFolderMap, sessionId, [], [folderId]) });
          } else {
            const conflicts = await addSessionToFolderWithConflicts(sessionId, folderId, current, folders);
            trackSessionMovedToFolder();
            set({ sessionFolderMap: updateSessionFolderMap(get().sessionFolderMap, sessionId, [folderId], conflicts) });
          }
          const name = get().folders.find(f => f.id === folderId)?.name ?? "folder";
          toast.success(isRemoving ? `Removed from ${name}` : `Added to ${name}`);
        } catch (e) {
          console.error("Failed to toggle session folder:", e);
          toast.error("Failed to update folder");
        }
      },

      addSessionToFolder: async (sessionId: string, folderId: string) => {
        try {
          const { sessionFolderMap, folders } = get();
          const current = sessionFolderMap[sessionId] ?? [];
          const conflicts = await addSessionToFolderWithConflicts(sessionId, folderId, current, folders);
          trackSessionMovedToFolder();
          set({ sessionFolderMap: updateSessionFolderMap(get().sessionFolderMap, sessionId, [folderId], conflicts) });
          const name = get().folders.find(f => f.id === folderId)?.name ?? "folder";
          toast.success(`Added to ${name}`);
        } catch (e) {
          console.error("Failed to add session to folder:", e);
          toast.error("Failed to add to folder");
        }
      },

      removeSessionFromAllFolders: async (sessionId: string) => {
        try {
          await dbRemoveSessionFromAllFolders(sessionId);
          const { [sessionId]: _, ...restMap } = get().sessionFolderMap;
          set({ sessionFolderMap: restMap });
          toast.success("Removed from all folders");
        } catch (e) {
          console.error("Failed to remove from folders:", e);
          toast.error("Failed to remove from folders");
        }
      },

      // Segment editing
      editSegmentText: async (segmentId: string, newText: string) => {
        try {
          await dbUpdateSegmentText(segmentId, newText);
          await get().refreshViewSessionSegments();
        } catch (e) {
          console.error("Failed to edit segment:", e);
          toast.error("Failed to edit segment");
        }
      },

      deleteSegment: async (segmentId: string) => {
        try {
          await dbSoftDeleteSegment(segmentId);
          await get().refreshViewSessionSegments();
        } catch (e) {
          console.error("Failed to delete segment:", e);
          toast.error("Failed to delete segment");
        }
      },

      toggleSegmentHidden: async (segmentId: string) => {
        try {
          await dbToggleSegmentHidden(segmentId);
          await get().refreshViewSessionSegments();
        } catch (e) {
          console.error("Failed to toggle segment visibility:", e);
          toast.error("Failed to toggle segment visibility");
        }
      },

      setShowHiddenSegments: (show: boolean) => {
        set({ showHiddenSegments: show });
      },

      refreshViewSessionSegments: async () => {
        const { selectedSessionId, activeSessionId } = get();
        if (!selectedSessionId || selectedSessionId === activeSessionId) return;
        try {
          const segments = await getSessionSegments(selectedSessionId);
          set({ viewSessionSegments: segments });
        } catch (e) {
          console.error("Failed to refresh segments:", e);
        }
      },

      // Note refresh
      incrementNoteRefresh: () =>
        set((state) => ({ noteRefreshCounter: state.noteRefreshCounter + 1 })),

      // Audio playback
      setPlaybackTime: (time: number) => set({ playbackTime: time }),
      setIsPlaying: (playing: boolean) => set({ isPlaying: playing }),

      // Dictation history
      loadDictationHistory: async () => {
        try {
          const history = await listDictationHistory();
          set({ dictationHistory: history });
        } catch (e) {
          console.error("Failed to load dictation history:", e);
        }
      },

      deleteDictationHistoryEntry: async (id: string) => {
        try {
          const entry = await getDictationHistoryEntry(id);
          if (entry?.wav_file_path) {
            commands.deleteSessionWav(id, null).catch(() => {});
          }
          await dbDeleteDictationHistoryEntry(id);
          set({ dictationHistory: get().dictationHistory.filter((h) => h.id !== id) });
        } catch (e) {
          console.error("Failed to delete dictation history entry:", e);
          toast.error("Failed to delete entry");
        }
      },

      clearDictationHistory: async () => {
        try {
          // Clean up WAV files for all entries — dictation WAVs always use default directory
          for (const entry of get().dictationHistory) {
            if (entry.wav_file_path) {
              commands.deleteSessionWav(entry.id, null).catch(() => {});
            }
          }
          await dbClearDictationHistory();
          set({ dictationHistory: [] });
        } catch (e) {
          console.error("Failed to clear dictation history:", e);
          toast.error("Failed to clear history");
        }
      },

      toggleSidebar: () => {
        set((state) => ({
          settings: {
            ...state.settings,
            sidebarCollapsed: !state.settings.sidebarCollapsed,
          },
        }));
      },

      completeFlow: (flowId: string) => {
        set((state) => ({
          settings: {
            ...state.settings,
            onboarding: {
              ...state.settings.onboarding,
              completedFlows: {
                ...state.settings.onboarding?.completedFlows,
                [flowId]: new Date().toISOString(),
              },
            },
          },
        }));
      },

      setUpdateAvailable: (update) => {
        const { updateDismissedVersion } = get();
        const dismissed =
          update && updateDismissedVersion === update.version
            ? updateDismissedVersion
            : null;
        set({ updateAvailable: update, updateDismissedVersion: dismissed });
      },
      dismissUpdate: () => {
        const { updateAvailable } = get();
        if (updateAvailable) {
          set({ updateDismissedVersion: updateAvailable.version });
        }
      },

      createManualNote: async (title?: string) => {
        try {
          const sessionId = crypto.randomUUID();
          await dbCreateManualSession(sessionId, title || "Untitled Note");
          trackManualNoteCreated();
          const sessions = await listSessions();
          const session = await getSession(sessionId);
          set({
            sessions,
            currentView: "note-detail",
            selectedSessionId: sessionId,
            viewSession: session,
            viewSessionSegments: [],
          });
        } catch (e) {
          console.error("Failed to create manual note:", e);
          toast.error("Failed to create note");
        }
      },
    }),
    {
      name: "yapstack-settings",
      version: 23,
      partialize: (state) => ({
        settings: state.settings,
      }),
      migrate: (persisted: unknown, version: number) => {
        const state = persisted as { settings?: Record<string, unknown> };
        if (version < 1 && state.settings) {
          // Merge graceSeconds → backfillSeconds, drop captureHistorySeconds
          const old = state.settings as Record<string, unknown>;
          if (old.backfillSeconds === undefined) {
            old.backfillSeconds =
              (old.graceSeconds as number | undefined) ?? 30;
          }
          delete old.graceSeconds;
          delete old.captureHistorySeconds;
        }
        if (version < 2 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.silenceDurationMs === undefined) {
            old.silenceDurationMs = defaultSettings.silenceDurationMs;
          }
          if (old.maxChunkSeconds === undefined) {
            old.maxChunkSeconds = defaultSettings.maxChunkSeconds;
          }
        }
        if (version < 3 && state.settings) {
          // v2 shipped with aggressive defaults that hurt quality — reset to proven values
          const old = state.settings as Record<string, unknown>;
          if (old.silenceDurationMs === 500) {
            old.silenceDurationMs = defaultSettings.silenceDurationMs;
          }
          if (old.maxChunkSeconds === 15) {
            old.maxChunkSeconds = defaultSettings.maxChunkSeconds;
          }
        }
        if (version < 4 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.promptContextChars === undefined) {
            old.promptContextChars = defaultSettings.promptContextChars;
          }
        }
        if (version < 5 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.theme === undefined) {
            old.theme = defaultSettings.theme;
          }
        }
        if (version < 6 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.sidebarCollapsed === undefined) {
            old.sidebarCollapsed = defaultSettings.sidebarCollapsed;
          }
        }
        if (version < 7 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.bufferMaxSeconds === undefined) {
            old.bufferMaxSeconds = 300;
          }
          delete old.backfillSeconds;
        }
        if (version < 8 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.ai === undefined) {
            old.ai = DEFAULT_AI_SETTINGS;
          }
        }
        if (version < 9 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.shortcutBindings === undefined) {
            old.shortcutBindings = {};
          }
        }
        if (version < 10 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.audioSaveLocation === undefined) {
            old.audioSaveLocation = null;
          }
        }
        if (version < 11 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.dictation === undefined) {
            old.dictation = DEFAULT_DICTATION_SETTINGS;
          }
        }
        if (version < 12 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          const dict = old.dictation as { slots?: Array<Record<string, unknown>> } | undefined;
          if (dict?.slots) {
            for (const slot of dict.slots) {
              if (slot.outputAction === undefined) {
                slot.outputAction = "paste";
              }
            }
          }
        }
        if (version < 13 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.showRecordingIndicator === undefined) {
            old.showRecordingIndicator = true;
          }
        }
        if (version < 14 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.selectedModelSize === "Base") {
            old.selectedModelSize = "Small";
          }
          if (old.captureSource === "MicOnly") {
            old.captureSource = "Mixed";
          }
        }
        if (version < 15 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.promptDecaySilenceSeconds === undefined) {
            old.promptDecaySilenceSeconds = defaultSettings.promptDecaySilenceSeconds;
          }
        }
        if (version < 16 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          const dict = old.dictation as { activationMode?: string } | undefined;
          if (dict && dict.activationMode === undefined) {
            dict.activationMode = "hold";
          }
        }
        if (version < 17 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          // Existing users have already configured the app — skip onboarding
          old.onboardingCompleted = true;
        }
        if (version < 18 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          old.onboarding = {
            completedFlows: old.onboardingCompleted
              ? { initial: new Date().toISOString() }
              : {},
          };
          delete old.onboardingCompleted;
        }
        if (version < 19 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          const dict = old.dictation as { slots?: Array<Record<string, unknown>> } | undefined;
          if (dict?.slots) {
            const slot1 = dict.slots.find((s) => s.id === "1");
            if (slot1 && slot1.defaultBinding === undefined) {
              slot1.defaultBinding = "Control+Shift+Space";
            }
          }
        }
        if (version < 20 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          // Replace name-based selectedMicDevice with ID-based selectedMicDeviceId.
          // Reset to null (system default) — user re-picks once on upgrade.
          delete old.selectedMicDevice;
          old.selectedMicDeviceId = null;
        }
        if (version < 21 && state.settings) {
          const old = state.settings as Record<string, unknown>;
          if (old.audioExportFormat === undefined) old.audioExportFormat = "mp3";
          if (old.mp3Bitrate === undefined) old.mp3Bitrate = 64;
        }
        if (version < 22 && state.settings) {
          // Engines become first-class peers. Default existing users to
          // Whisper so behavior is unchanged on upgrade; the cascading
          // settings UI lets them switch.
          const old = state.settings as Record<string, unknown>;
          if (old.selectedEngine === undefined) old.selectedEngine = "Whisper";
          if (old.selectedParakeetVariant === undefined)
            old.selectedParakeetVariant = "TdtV3";
          if (old.diarizationEnabled === undefined)
            old.diarizationEnabled = false;
          if (old.speakerNames === undefined) old.speakerNames = {};
        }
        if (version < 23 && state.settings) {
          // Force-disable diarization on upgrade. Sortformer's chunk-local
          // speaker IDs cause the same person to flip across speaker
          // numbers across chunk boundaries. We're keeping the IPC + DB +
          // sidecar plumbing intact so re-enable is one line away once
          // session-stable IDs land, but no user should be running with
          // the broken behavior in the meantime.
          const old = state.settings as Record<string, unknown>;
          old.diarizationEnabled = false;
        }
        return state as unknown as { settings: Settings };
      },
    },
  ),
);
}

// Pin the store into import.meta.hot.data so Vite HMR preserves runtime state
// (enginePhase, captureStatus, active session, etc.) across module reloads.
// Without this, every HMR tick resets enginePhase to "idle" until autoSetup's
// backend probe completes, causing dictation and other engine-gated features
// to flicker. See the HMR rehydration research notes for context.
const HMR_STORE_KEY = "__yapstack_app_store__";
type HmrStore = ReturnType<typeof createAppStore>;

function getOrCreateAppStore(): HmrStore {
  // In vitest, import.meta.hot exists but .data may be undefined. Fall back
  // to a fresh store in that case — HMR-persistence is a dev-server concern
  // that doesn't apply to test isolation.
  const hot = import.meta.hot?.data as
    | Record<string, HmrStore>
    | undefined;
  if (!hot) return createAppStore();
  if (!hot[HMR_STORE_KEY]) {
    hot[HMR_STORE_KEY] = createAppStore();
  }
  return hot[HMR_STORE_KEY];
}

export const useAppStore = getOrCreateAppStore();
