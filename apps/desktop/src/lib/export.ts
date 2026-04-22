import type { DbSegment } from "@/lib/db";

/** Concatenates segments as plain text, one per line, in time order. */
export function segmentsToPlainText(segments: DbSegment[]): string {
  return [...segments]
    .sort((a, b) => a.audio_offset_seconds - b.audio_offset_seconds)
    .map((s) => s.text.trim())
    .filter((t) => t.length > 0)
    .join("\n");
}

function formatSrtTime(seconds: number): string {
  const total = Math.max(0, seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = Math.floor(total % 60);
  const ms = Math.round((total - Math.floor(total)) * 1000);
  const pad = (n: number, w = 2) => n.toString().padStart(w, "0");
  return `${pad(h)}:${pad(m)}:${pad(s)},${pad(ms, 3)}`;
}

/** Serializes segments as an SRT subtitle file. */
export function segmentsToSrt(segments: DbSegment[]): string {
  const sorted = [...segments].sort(
    (a, b) => a.audio_offset_seconds - b.audio_offset_seconds,
  );
  return sorted
    .map((s, i) => {
      const start = s.audio_offset_seconds;
      const end = start + (s.chunk_duration_seconds || 1);
      return `${i + 1}\n${formatSrtTime(start)} --> ${formatSrtTime(end)}\n${s.text.trim()}\n`;
    })
    .join("\n");
}

/** Triggers a browser download of `content` under `filename`. */
export function downloadTextFile(filename: string, content: string) {
  const blob = new Blob([content], { type: "text/plain;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}
