import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  formatBytes,
  formatDuration,
  formatTime,
  formatElapsed,
  formatShortcutDisplay,
  formatRelativeTime,
  getDayLabel,
  groupSessionsByDay,
  cn,
} from "./utils";

describe("formatBytes", () => {
  it("returns 0 B for zero", () => {
    expect(formatBytes(0)).toBe("0 B");
  });

  it("returns bytes for values < 1024", () => {
    expect(formatBytes(512)).toBe("512 B");
  });

  it("returns KB for values < 1 MB", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(2560)).toBe("2.5 KB");
  });

  it("returns MB for values < 1 GB", () => {
    expect(formatBytes(1024 * 1024)).toBe("1 MB");
    expect(formatBytes(150 * 1024 * 1024)).toBe("150 MB");
  });

  it("returns GB for large values", () => {
    expect(formatBytes(1024 * 1024 * 1024)).toBe("1.0 GB");
    expect(formatBytes(1.5 * 1024 * 1024 * 1024)).toBe("1.5 GB");
  });

  it("handles fractional MB correctly", () => {
    // 500 KB = ~0.5 MB, rounds to 0 MB
    expect(formatBytes(500 * 1024)).toBe("500.0 KB");
  });
});

describe("formatDuration", () => {
  it("formats 0 seconds", () => {
    expect(formatDuration(0)).toBe("0s");
  });

  it("formats seconds only", () => {
    expect(formatDuration(45)).toBe("45s");
  });

  it("formats exact minutes", () => {
    expect(formatDuration(60)).toBe("1m 0s");
  });

  it("formats minutes and seconds", () => {
    expect(formatDuration(150)).toBe("2m 30s");
  });

  it("handles fractional seconds by flooring", () => {
    expect(formatDuration(90.7)).toBe("1m 30s");
  });
});

describe("formatTime", () => {
  it("formats 0 as 0:00", () => {
    expect(formatTime(0)).toBe("0:00");
  });

  it("pads seconds to two digits", () => {
    expect(formatTime(5)).toBe("0:05");
  });

  it("formats minutes and seconds", () => {
    expect(formatTime(65)).toBe("1:05");
  });

  it("formats 10 minutes", () => {
    expect(formatTime(600)).toBe("10:00");
  });

  it("wraps at 60 minutes into hours bucket", () => {
    expect(formatTime(3600)).toBe("1:00:00");
  });

  it("formats hours, minutes, and seconds together", () => {
    expect(formatTime(3725)).toBe("1:02:05");
  });

  it("formats values past two hours", () => {
    expect(formatTime(7384)).toBe("2:03:04");
  });
});

describe("formatElapsed", () => {
  it("formats 0 ms as 00:00", () => {
    expect(formatElapsed(0)).toBe("00:00");
  });

  it("formats 5 seconds", () => {
    expect(formatElapsed(5000)).toBe("00:05");
  });

  it("formats 2.5 minutes", () => {
    expect(formatElapsed(150000)).toBe("02:30");
  });

  it("wraps at 60 minutes into hours bucket", () => {
    expect(formatElapsed(3600000)).toBe("01:00:00");
  });

  it("formats hours, minutes, and seconds together", () => {
    expect(formatElapsed(3725000)).toBe("01:02:05");
  });

  it("formats values past two hours", () => {
    expect(formatElapsed(7384000)).toBe("02:03:04");
  });
});

describe("formatShortcutDisplay", () => {
  // jsdom doesn't set Mac userAgent, so isMac is false (non-Mac path)
  it("formats mod+k for non-Mac", () => {
    expect(formatShortcutDisplay("mod+k")).toBe("Ctrl+K");
  });

  it("formats mod+shift+n for non-Mac", () => {
    expect(formatShortcutDisplay("mod+shift+n")).toBe("Ctrl+Shift+N");
  });

  it("formats escape for non-Mac", () => {
    expect(formatShortcutDisplay("escape")).toBe("Esc");
  });

  it("formats alt+p for non-Mac", () => {
    expect(formatShortcutDisplay("alt+p")).toBe("Alt+P");
  });

  it("formats backspace for non-Mac", () => {
    expect(formatShortcutDisplay("mod+backspace")).toBe("Ctrl+Backspace");
  });

  it("handles single key", () => {
    expect(formatShortcutDisplay("f")).toBe("F");
  });
});

describe("cn", () => {
  it("merges class names", () => {
    expect(cn("foo", "bar")).toBe("foo bar");
  });

  it("handles conditional classes", () => {
    const condition = false;
    expect(cn("foo", condition && "bar", "baz")).toBe("foo baz");
  });
});

describe("formatRelativeTime", () => {
  const FIXED_NOW = new Date("2025-06-15T12:00:00Z");

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(FIXED_NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns "Just now" for recent timestamps', () => {
    expect(formatRelativeTime("2025-06-15T11:59:50")).toBe("Just now");
  });

  it("returns minutes ago for < 1 hour", () => {
    expect(formatRelativeTime("2025-06-15T11:55:00")).toBe("5m ago");
  });

  it("returns hours ago for < 24 hours", () => {
    expect(formatRelativeTime("2025-06-15T09:00:00")).toBe("3h ago");
  });

  it("returns days ago for < 7 days", () => {
    expect(formatRelativeTime("2025-06-13T12:00:00")).toBe("2d ago");
  });

  it("returns formatted date for >= 7 days", () => {
    const result = formatRelativeTime("2025-06-05T12:00:00");
    expect(result).toMatch(/\w+ \d+/);
  });
});

describe("getDayLabel", () => {
  const FIXED_NOW = new Date("2025-06-15T12:00:00Z");

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(FIXED_NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns "Today" for today\'s date', () => {
    expect(getDayLabel("2025-06-15T10:00:00")).toBe("Today");
  });

  it('returns "Yesterday" for yesterday\'s date', () => {
    expect(getDayLabel("2025-06-14T10:00:00")).toBe("Yesterday");
  });

  it("returns formatted date for older dates", () => {
    const result = getDayLabel("2025-06-10T10:00:00");
    expect(result).not.toBe("Today");
    expect(result).not.toBe("Yesterday");
    expect(result.length).toBeGreaterThan(0);
  });
});

describe("groupSessionsByDay", () => {
  const FIXED_NOW = new Date("2025-06-15T12:00:00Z");

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(FIXED_NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns empty array for no sessions", () => {
    expect(groupSessionsByDay([])).toEqual([]);
  });

  it("groups sessions with same date under one label", () => {
    const sessions = [
      { id: "1", created_at: "2025-06-15T10:00:00" },
      { id: "2", created_at: "2025-06-15T11:00:00" },
    ];
    const groups = groupSessionsByDay(sessions);
    expect(groups.length).toBe(1);
    expect(groups[0].sessions.length).toBe(2);
  });

  it("creates separate groups for different days", () => {
    const sessions = [
      { id: "1", created_at: "2025-06-15T10:00:00" },
      { id: "2", created_at: "2025-06-14T10:00:00" },
    ];
    const groups = groupSessionsByDay(sessions);
    expect(groups.length).toBe(2);
    expect(groups[0].label).toBe("Today");
    expect(groups[1].label).toBe("Yesterday");
  });

  it("preserves session order within groups", () => {
    const sessions = [
      { id: "a", created_at: "2025-06-15T10:00:00" },
      { id: "b", created_at: "2025-06-15T11:00:00" },
      { id: "c", created_at: "2025-06-15T12:00:00" },
    ];
    const groups = groupSessionsByDay(sessions);
    expect(groups[0].sessions.map((s) => s.id)).toEqual(["a", "b", "c"]);
  });
});
