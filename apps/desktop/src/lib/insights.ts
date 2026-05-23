/**
 * Live insights — user-defined LLM extractions that run against the live
 * session transcript on a heartbeat and render their output in the floating
 * Insight overlay.
 *
 * Each Insight bundles a prompt + heartbeat + AI Profile binding. Only one
 * Insight is active at a time (the Active Insight); inactive insights are
 * dormant and consume no LLM calls.
 *
 * See docs/UBIQUITOUS_LANGUAGE.md "Live insights" section for the canonical
 * vocabulary (Insight, Active Insight, Insight heartbeat, Insight tick,
 * Insight result, Insight overlay, Insight backfill).
 */

export interface Insight {
  id: string;                   // uuid
  name: string;                 // user label, duplicates allowed
  enabled: boolean;             // eligible to be the Active Insight
  profileId: string | null;     // → AI Profile id; null = unconfigured
  prompt: string;               // user-editable system prompt
  heartbeatSeconds: number;     // interval; 15 / 30 / 60 / 120 / custom
}

export interface InsightsSettings {
  enabled: boolean;             // master feature toggle (persisted)
  /** What auto-loads as the Current Insight at the start of each session.
   *  Persisted. Edited only from Settings → AI → Insights. Setting this to
   *  `null` means "no auto-start" — sessions begin with no overlay; the
   *  user can pick a Current Insight via the overlay header dropdown if
   *  the overlay is already visible. */
  defaultInsightId: string | null;
  slots: Insight[];
}

export const DEFAULT_INSIGHTS_SETTINGS: InsightsSettings = {
  enabled: false,
  defaultInsightId: null,
  slots: [],
};

/** Whether the Insight overlay should be visible right now. The overlay must
 *  ONLY show when the feature is on, a session is live, AND the Current
 *  Insight resolves to a slot that still exists and is enabled. Checking
 *  `currentInsightId !== null` alone is not enough: a Default Insight that was
 *  later disabled (the Default picker keeps the stored id) or an Insight
 *  deleted/disabled mid-session would leave a "zombie" overlay the engine can
 *  never feed — it gates off on the same missing/disabled condition. Keep this
 *  in lockstep with the engine gate in `useInsightEngine`. */
export function shouldShowInsightOverlay(params: {
  enabled: boolean;
  liveTranscriptionActive: boolean;
  currentInsightId: string | null;
  slots: Insight[];
}): boolean {
  const { enabled, liveTranscriptionActive, currentInsightId, slots } = params;
  if (!enabled || !liveTranscriptionActive || !currentInsightId) return false;
  const slot = slots.find((s) => s.id === currentInsightId);
  return !!slot?.enabled;
}

/** Cadence options surfaced in the UI. Custom values can still be persisted. */
export const HEARTBEAT_PRESETS: { value: number; label: string }[] = [
  { value: 15, label: "15s" },
  { value: 30, label: "30s" },
  { value: 60, label: "60s" },
  { value: 120, label: "2m" },
];

export const DEFAULT_HEARTBEAT_SECONDS = 30;

export function buildInsight(name: string): Insight {
  return {
    id: crypto.randomUUID(),
    name: name.trim() || "New Insight",
    enabled: true,
    profileId: null,
    prompt: "",
    heartbeatSeconds: DEFAULT_HEARTBEAT_SECONDS,
  };
}

/** Runtime state for the currently-rendering Insight result. Not persisted. */
export interface LiveInsightResult {
  insightId: string;
  content: string;
  generatedAt: string;          // ISO
  segmentCountAtRun: number;    // watermark for skip-if-no-new-content
}

/** Sentinel rendered when the LLM returns an empty completion for a tick.
 *  Kept as a distinct constant so the engine knows to NOT pass this back
 *  into the next prompt's `<previous>` block — it's a UI placeholder, not
 *  content the model would benefit from seeing. */
export const EMPTY_RESULT_PLACEHOLDER = "_(no output for this window)_";

/** Hard cap on how much previous content we ship in the prompt context.
 *  Most insights produce 2-3 sentences (≲ 500 chars); this is a defensive
 *  ceiling for prompts that accumulate (e.g. append-style lists). When the
 *  previous content exceeds the cap, we keep the LAST N chars — recency
 *  matters more than the oldest items. */
export const PREVIOUS_CONTEXT_MAX_CHARS = 1500;

export type LiveInsightStatus = "idle" | "running" | "error";
