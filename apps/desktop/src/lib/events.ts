import { listen, emit } from "@tauri-apps/api/event";
import type {
  CaptureStatusDto,
  BufferStatusDto,
  LiveTranscriptionStatus,
} from "@/lib/tauri";
import type { LiveSegmentEvent } from "@/lib/tauri";

// --- Payload types for events not covered by specta-generated DTOs ---

export type ModelDownloadProgress = {
  percent: number;
  size: string;
};

export type SessionPartReadyEvent = {
  session_id: string;
  part_index: number;
  file_path: string;
  format: "wav" | "mp3";
  duration_seconds: number;
  sample_rate: number;
};

export type SessionWavErrorEvent = {
  session_id: string;
  message: string;
};

export type SessionWavWarningEvent = {
  session_id: string;
  message: string;
};

export type LiveTranscriptionWarningEvent = {
  message: string;
};

export type StreamHealthEvent = {
  source: "Mic" | "System";
  status: "restarted" | "restart_failed" | "restart_abandoned";
  message: string;
};

/**
 * Per-chunk transcription pressure telemetry. Fires after every transcribe
 * attempt (success or failure). RTFx < 1 means the pipeline is slower than
 * real time; sustained low RTFx + rising lag is the failure mode that Stage 1
 * exists to surface.
 */
export type LiveTranscriptionPressureEvent = {
  source: "Mic" | "System";
  chunk_index: number;
  chunk_audio_seconds: number;
  wall_ms: number;
  /** Null when the transcribe call failed or wall_ms was 0. */
  rtfx: number | null;
  engine: "Whisper" | "Parakeet";
  is_backfill: boolean;
  /** Null when the chunk did not produce a successful transcription. */
  lag_seconds: number | null;
};

export type BubbleState =
  | "recording" | "transcribing" | "processing"
  | "pasted" | "copied" | "note-created"
  | "no-speech" | "no-input" | "error" | "cancelled";

export type DictationStateEvent = {
  state: BubbleState;
  slotName?: string;
};

// --- Centralized window label constants ---

export const WINDOWS = {
  MAIN: "main",
  DICTATION: "dictation",
  RECORDING_INDICATOR: "recording-indicator",
} as const;

// --- Centralized event name constants ---

export const EVENTS = {
  // Capture
  CAPTURE_STATUS: "capture-status",
  BUFFER_INFO: "buffer-info",

  // Live transcription
  LIVE_TRANSCRIPTION_SEGMENT: "live-transcription-segment",
  LIVE_TRANSCRIPTION_STATUS: "live-transcription-status",
  LIVE_TRANSCRIPTION_WARNING: "live-transcription-warning",
  LIVE_TRANSCRIPTION_PRESSURE: "live-transcription-pressure",
  BACKFILL_COMPLETE: "backfill-complete",
  SESSION_PART_READY: "session-part-ready",
  SESSION_WAV_ERROR: "session-wav-error",
  SESSION_WAV_WARNING: "session-wav-warning",
  STREAM_HEALTH: "stream-health",

  // Model
  MODEL_DOWNLOAD_PROGRESS: "model-download-progress",

  // Tray
  TRAY_NEW_SESSION: "tray:new-session",
  TRAY_NEW_SESSION_ALL: "tray:new-session-all",
  TRAY_STOP_SESSION: "tray:stop-session",

  // Dictation
  DICTATION_STATE: "dictation:state",

  // Recording indicator
  RECORDING_INDICATOR_ACTIVE: "recording-indicator:active",
  RECORDING_INDICATOR_OPEN_MAIN: "recording-indicator:open-main",
} as const;

// --- Type-safe event payload map ---

type EventPayloadMap = {
  [EVENTS.CAPTURE_STATUS]: CaptureStatusDto;
  [EVENTS.BUFFER_INFO]: BufferStatusDto;
  [EVENTS.LIVE_TRANSCRIPTION_SEGMENT]: LiveSegmentEvent;
  [EVENTS.LIVE_TRANSCRIPTION_STATUS]: LiveTranscriptionStatus;
  [EVENTS.LIVE_TRANSCRIPTION_WARNING]: LiveTranscriptionWarningEvent;
  [EVENTS.LIVE_TRANSCRIPTION_PRESSURE]: LiveTranscriptionPressureEvent;
  [EVENTS.BACKFILL_COMPLETE]: void;
  [EVENTS.SESSION_PART_READY]: SessionPartReadyEvent;
  [EVENTS.SESSION_WAV_ERROR]: SessionWavErrorEvent;
  [EVENTS.SESSION_WAV_WARNING]: SessionWavWarningEvent;
  [EVENTS.STREAM_HEALTH]: StreamHealthEvent;
  [EVENTS.MODEL_DOWNLOAD_PROGRESS]: ModelDownloadProgress;
  [EVENTS.TRAY_NEW_SESSION]: number;
  [EVENTS.TRAY_NEW_SESSION_ALL]: void;
  [EVENTS.TRAY_STOP_SESSION]: void;
  [EVENTS.DICTATION_STATE]: DictationStateEvent;
  [EVENTS.RECORDING_INDICATOR_ACTIVE]: boolean;
  [EVENTS.RECORDING_INDICATOR_OPEN_MAIN]: void;
};

type EventName = keyof EventPayloadMap;

/**
 * Type-safe wrapper around Tauri's `listen`. Enforces the correct payload type
 * for each event name.
 */
export function listenEvent<E extends EventName>(
  event: E,
  handler: (payload: EventPayloadMap[E]) => void,
) {
  return listen<EventPayloadMap[E]>(event, (e) => handler(e.payload));
}

/**
 * Type-safe wrapper around Tauri's `emit`. Enforces the correct payload type
 * for each event name.
 */
export function emitEvent<E extends EventName>(
  event: E,
  ...args: EventPayloadMap[E] extends void ? [] : [EventPayloadMap[E]]
) {
  return emit(event, args[0]);
}
