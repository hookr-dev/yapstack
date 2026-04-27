export { commands } from "./types";
export type * from "./types";

export type AudioSourceLabel = "Mic" | "System";

/**
 * Mirror of the Rust `TranscriptSegmentDto` (in
 * `commands/transcription.rs`). Specta only emits types that are reachable
 * from a registered command's args/return; since live segments arrive via
 * the `live-transcription-segment` event surface (not a command), we declare
 * the shape locally here to keep the FE event type self-contained.
 */
export type TranscriptSegmentDto = {
  start_ms: number;
  end_ms: number;
  text: string;
  confidence: number;
  /** Populated when the active engine is Parakeet *and* diarization was
   *  requested for the originating transcribe call. `null` for Whisper. */
  speaker_id: number | null;
};

export type LiveSegmentEvent = {
  chunk_index: number;
  source: AudioSourceLabel;
  segments: TranscriptSegmentDto[];
  audio_offset_seconds: number;
  chunk_duration_seconds: number;
  accumulated_text: string;
  is_backfill: boolean;
  /** Session this chunk belongs to. Late-arriving segments still persist to
   * this session even after the frontend has cleared activeSessionId. */
  session_id: string | null;
};
