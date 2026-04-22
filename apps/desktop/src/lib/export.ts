import type { DbSegment } from "@/lib/db";

/** Concatenates segments as plain text, one per line, in time order. */
export function segmentsToPlainText(segments: DbSegment[]): string {
  return [...segments]
    .sort((a, b) => a.audio_offset_seconds - b.audio_offset_seconds)
    .map((s) => s.text.trim())
    .filter((t) => t.length > 0)
    .join("\n");
}
