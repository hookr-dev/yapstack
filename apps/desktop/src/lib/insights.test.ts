import { describe, it, expect } from "vitest";
import { buildInsight, shouldShowInsightOverlay, type Insight } from "./insights";

function insight(overrides: Partial<Insight> = {}): Insight {
  return { ...buildInsight("Test"), ...overrides };
}

describe("shouldShowInsightOverlay", () => {
  const enabledSlot = insight({ id: "a", enabled: true });

  it("shows when feature on, session live, and the Current Insight is enabled", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: true,
        liveTranscriptionActive: true,
        currentInsightId: "a",
        slots: [enabledSlot],
      }),
    ).toBe(true);
  });

  it("hides when the feature is off", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: false,
        liveTranscriptionActive: true,
        currentInsightId: "a",
        slots: [enabledSlot],
      }),
    ).toBe(false);
  });

  it("hides when no session is live", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: true,
        liveTranscriptionActive: false,
        currentInsightId: "a",
        slots: [enabledSlot],
      }),
    ).toBe(false);
  });

  it("hides when there is no Current Insight", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: true,
        liveTranscriptionActive: true,
        currentInsightId: null,
        slots: [enabledSlot],
      }),
    ).toBe(false);
  });

  // Regression: a Default Insight that was later disabled still seeds
  // currentInsightId at session start, but the engine won't feed a disabled
  // slot — so the overlay must NOT show (no zombie "Waiting…" overlay).
  it("hides when the Current Insight exists but is disabled", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: true,
        liveTranscriptionActive: true,
        currentInsightId: "a",
        slots: [insight({ id: "a", enabled: false })],
      }),
    ).toBe(false);
  });

  // Regression: deleting the running Insight mid-session leaves a dangling
  // currentInsightId; the gate must hide rather than keep a stale overlay.
  it("hides when the Current Insight no longer exists in slots", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: true,
        liveTranscriptionActive: true,
        currentInsightId: "gone",
        slots: [enabledSlot],
      }),
    ).toBe(false);
  });
});
