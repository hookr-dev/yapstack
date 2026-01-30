export { commands } from "./types";
export type * from "./types";

import type { TranscriptSegmentDto } from "./types";

export type AudioSourceLabel = "Mic" | "System";

export type LiveSegmentEvent = {
  chunk_index: number;
  source: AudioSourceLabel;
  segments: TranscriptSegmentDto[];
  audio_offset_seconds: number;
  chunk_duration_seconds: number;
  accumulated_text: string;
  is_backfill: boolean;
};
