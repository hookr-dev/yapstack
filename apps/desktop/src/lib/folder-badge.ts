import type { CSSProperties } from "react";

/**
 * Inline style for a folder-tinted pill: a low-opacity wash of the folder's
 * color for the background plus the same color for foreground/icon. Used by
 * NoteCard's session→folder chips and the in-session AutoTag suggestion pill
 * so both surfaces feel visually identical.
 */
export function folderBadgeStyle(color: string | null): CSSProperties {
  if (!color) return {};
  return {
    backgroundColor: `color-mix(in oklch, ${color} 15%, transparent)`,
    color: color,
  };
}
