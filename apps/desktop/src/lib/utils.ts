import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/** Format byte count to human-readable string (e.g. "142 MB", "1.5 GB"). */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Format seconds to a compact duration string (e.g. "2m 30s", "45s"). */
export function formatDuration(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  if (mins === 0) return `${secs}s`;
  return `${mins}m ${secs}s`;
}

/** Format seconds offset to MM:SS timestamp (e.g. "1:05"). */
export function formatTime(offsetSeconds: number): string {
  const mins = Math.floor(offsetSeconds / 60);
  const secs = Math.floor(offsetSeconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

/** Format elapsed milliseconds to MM:SS (e.g. "02:30"). */
export function formatElapsed(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const mins = Math.floor(totalSeconds / 60);
  const secs = totalSeconds % 60;
  return `${mins.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}`;
}

/** Format a date string to a relative time label (e.g. "Just now", "5m ago", "2d ago"). */
export function formatRelativeTime(dateStr: string): string {
  const date = new Date(dateStr + "Z");
  const diff = Date.now() - date.getTime();
  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return "Just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

/** Get a human-readable day label for a date string (e.g. "Today", "Yesterday"). */
export function getDayLabel(dateStr: string): string {
  const date = new Date(dateStr + "Z");
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const sessionDay = new Date(
    date.getFullYear(),
    date.getMonth(),
    date.getDate(),
  );
  const diffDays = Math.floor(
    (today.getTime() - sessionDay.getTime()) / 86400000,
  );

  if (diffDays === 0) return "Today";
  if (diffDays === 1) return "Yesterday";
  return date.toLocaleDateString(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
  });
}

interface SessionGroup<T> {
  label: string;
  sessions: T[];
}

/** Group sessions by day label using their `created_at` field. */
export function groupSessionsByDay<T extends { created_at: string }>(
  sessions: T[],
): SessionGroup<T>[] {
  const groups: SessionGroup<T>[] = [];
  let currentLabel = "";
  for (const session of sessions) {
    const label = getDayLabel(session.created_at);
    if (label !== currentLabel) {
      currentLabel = label;
      groups.push({ label, sessions: [session] });
    } else {
      groups[groups.length - 1].sessions.push(session);
    }
  }
  return groups;
}

/** Full capture source display labels. */
export const SOURCE_LABELS_FULL = {
  MicOnly: "Mic Only",
  SystemOnly: "System Only",
  Mixed: "Mixed",
} as const;

/** Whether the current platform is macOS. */
export const isMac = /Mac|iPod|iPhone|iPad/.test(navigator.userAgent);

/**
 * Convert a binding string like "mod+shift+n" to a display string like "⌘⇧N".
 * On non-Mac: "Ctrl+Shift+N".
 */
export function formatShortcutDisplay(binding: string): string {
  const parts = binding.toLowerCase().split("+");
  if (isMac) {
    const symbols: string[] = [];
    for (const p of parts) {
      if (p === "mod") symbols.push("⌘");
      else if (p === "shift") symbols.push("⇧");
      else if (p === "alt") symbols.push("⌥");
      else if (p === "backspace") symbols.push("⌫");
      else if (p === "escape") symbols.push("⎋");
      else symbols.push(p.toUpperCase());
    }
    return symbols.join("");
  }
  const labels: string[] = [];
  for (const p of parts) {
    if (p === "mod") labels.push("Ctrl");
    else if (p === "shift") labels.push("Shift");
    else if (p === "alt") labels.push("Alt");
    else if (p === "backspace") labels.push("Backspace");
    else if (p === "escape") labels.push("Esc");
    else labels.push(p.toUpperCase());
  }
  return labels.join("+");
}
