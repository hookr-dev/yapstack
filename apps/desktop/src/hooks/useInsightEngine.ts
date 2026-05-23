import { useEffect, useMemo, useRef } from "react";
import { useAppStore } from "@/stores/appStore";
import {
  assembleTranscriptContext,
  resolveAndCreateClient,
} from "@/lib/ai";
import {
  EMPTY_RESULT_PLACEHOLDER,
  PREVIOUS_CONTEXT_MAX_CHARS,
} from "@/lib/insights";
import { log } from "@/lib/logger";

/** Output-format guidance appended to every Insight's system prompt by the
 *  engine. Keeps users' freeform insight prompts focused on *what* to extract
 *  while the engine handles *how* the result is shaped for the small overlay
 *  surface. Markdown rendering is enabled in the overlay
 *  (`.ai-chat-markdown` styles), so encouraging it improves readability.
 *  The 12-line ceiling is enforced here at the prompt layer — we
 *  intentionally do NOT pass `max_tokens` / `max_completion_tokens` to the
 *  completion call (see `testConnection` in `lib/ai.ts`: OpenAI's newer
 *  models reject the legacy `max_tokens`, and self-hosted OpenAI-compatible
 *  servers don't all recognise `max_completion_tokens` — leaving the cap
 *  off is the portable choice across providers). The overlay's
 *  `overflow-y-auto` body is the safety net if a model overshoots. */
const OUTPUT_FORMAT_GUIDANCE =
  "\n\nOutput format: respond in 12 lines or fewer. " +
  "Use compact Markdown where it helps — `**bold**` for key terms, `-` for " +
  "short bulleted lists. Avoid heading syntax (`#`); the output renders in " +
  "a small floating overlay window.";

/**
 * Insight engine — drives the Active Insight on its heartbeat while a live
 * session is running. Writes results to the Zustand store; the overlay
 * controller reads from there.
 *
 * Discipline:
 *  - Skip the tick if `activeSessionSegments.length` hasn't advanced since
 *    the last successful run (no LLM call on silence).
 *  - At most one in-flight call per Active Insight; subsequent ticks are
 *    dropped (no queue).
 *  - On Active Insight change OR on Active Insight's enabled/heartbeat
 *    change, reset the interval and the watermark so the next tick acts as
 *    a backfill against the session-so-far transcript.
 *  - Errors surface to `liveInsightError`; no auto-retry, no fallback chain
 *    (per the surface-errors design memory).
 */
export function useInsightEngine() {
  const enabled = useAppStore((s) => s.settings.insights.enabled);
  // The engine drives the Current Insight (runtime), not the Default (which
  // is persisted and only relevant at session start). Mid-session switches
  // via the overlay update `currentInsightId` directly; session-start
  // initialization copies Default → Current in the store lifecycle hooks.
  const currentInsightId = useAppStore((s) => s.currentInsightId);
  const liveTranscriptionActive = useAppStore((s) => s.liveTranscriptionActive);

  // Pull only the engine-relevant fields of the current insight into the dep
  // array. Subscribing to `slots` directly would re-run the effect on every
  // prompt keystroke; this surface (id + enabled + heartbeat) only changes
  // when the user toggles the slot or picks a new cadence.
  const slots = useAppStore((s) => s.settings.insights.slots);
  const currentInsight = useMemo(
    () => slots.find((s) => s.id === currentInsightId) ?? null,
    [slots, currentInsightId],
  );
  // Derive only the primitive fields the scheduler effect needs. Depending on
  // the whole `currentInsight` object would re-run the effect (tear down +
  // restart the interval, fire an immediate backfill tick) on every prompt
  // keystroke while the active Insight is being edited — a new object is
  // produced each edit even though id/enabled/heartbeat are unchanged.
  const currentInsightExists = currentInsight !== null;
  const currentEnabled = currentInsight?.enabled ?? false;
  const currentHeartbeatSeconds = currentInsight?.heartbeatSeconds ?? 30;

  const inFlightRef = useRef(false);
  const abortRef = useRef<AbortController | null>(null);
  const processedCountRef = useRef(0);

  // Reset the watermark AND clear the previous result/status when the Current
  // Insight changes. The next tick will run against the full session-so-far
  // transcript (the backfill semantics) without the OLD insight's content
  // bleeding into the new insight's prompt-context block or overlay render.
  useEffect(() => {
    processedCountRef.current = 0;
    const s = useAppStore.getState();
    s.setLiveInsightResult(null);
    s.setLiveInsightStatus("idle");
    s.setLiveInsightError(null);
  }, [currentInsightId]);

  useEffect(() => {
    if (!enabled || !liveTranscriptionActive || !currentInsightId) {
      log.debug(
        `engine: gate closed (enabled=${enabled}, live=${liveTranscriptionActive}, currentId=${currentInsightId ?? "null"})`,
        "insights",
      );
      return;
    }
    if (!currentInsightExists || !currentEnabled) {
      log.debug(
        `engine: current insight missing or disabled (found=${currentInsightExists}, enabled=${currentEnabled})`,
        "insights",
      );
      return;
    }

    const heartbeatMs = Math.max(1, currentHeartbeatSeconds) * 1000;
    // Read the name fresh for the log (the effect intentionally does not
    // re-run on rename/prompt edits, so a closed-over name could be stale).
    const startName =
      useAppStore
        .getState()
        .settings.insights.slots.find((s) => s.id === currentInsightId)?.name ??
      "Insight";
    log.info(
      `engine: starting — insight="${startName}" heartbeat=${currentHeartbeatSeconds}s`,
      "insights",
    );

    async function runTick() {
      if (inFlightRef.current) {
        log.debug("tick: skip — call already in flight", "insights");
        return;
      }

      const state = useAppStore.getState();
      if (!state.liveTranscriptionActive) {
        log.debug("tick: skip — live transcription not active", "insights");
        return;
      }

      const live = state.settings.insights;
      const liveCurrentId = state.currentInsightId;
      if (!live.enabled || liveCurrentId !== currentInsightId) {
        log.debug(
          `tick: skip — config changed (enabled=${live.enabled}, currentId=${liveCurrentId ?? "null"} expected=${currentInsightId})`,
          "insights",
        );
        return;
      }

      const insight = live.slots.find((s) => s.id === currentInsightId);
      if (!insight || !insight.enabled) {
        log.debug("tick: skip — insight missing or disabled", "insights");
        return;
      }

      const allSegments = state.activeSessionSegments;
      if (allSegments.length === processedCountRef.current) {
        log.debug(
          `tick: skip — no new segments since last run (watermark=${processedCountRef.current})`,
          "insights",
        );
        return;
      }

      let client;
      let model;
      try {
        ({ client, model } = resolveAndCreateClient(
          state.settings.aiConfig,
          insight.profileId,
        ));
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        log.warn(`tick: profile resolution failed — ${msg}`, "insights");
        state.setLiveInsightStatus("error");
        state.setLiveInsightError(msg);
        return;
      }

      // Only send segments that arrived since the last successful tick.
      // First tick (or post-switch backfill): watermark is 0, so the slice
      // covers the whole session-so-far. Subsequent ticks send just the
      // delta — paired with the <previous> block, the model has the
      // anchor it needs to refine vs pivot vs append without re-processing
      // the entire transcript every cycle.
      const newSegments = allSegments.slice(processedCountRef.current);
      const transcript = assembleTranscriptContext(newSegments);
      if (!transcript) {
        log.debug(
          `tick: skip — transcript empty after assembly (newSegments=${newSegments.length}, total=${allSegments.length})`,
          "insights",
        );
        return;
      }

      // Thread the previous result back into the prompt so the model can
      // decide whether the topic is continuing (refine) or has shifted
      // (start over). Filter out the empty-completion placeholder — that's
      // a UI artefact, not content worth showing the model. Also skip if
      // the cached result is for a different insight (defensive — the
      // active-insight effect above clears it on switch).
      const prevResult = state.liveInsightResult;
      const previousContent =
        prevResult &&
        prevResult.insightId === insight.id &&
        prevResult.content !== EMPTY_RESULT_PLACEHOLDER
          ? truncatePrevious(prevResult.content)
          : null;

      const userMessage = previousContent
        ? `<previous>\n${previousContent}\n</previous>\n\n<transcript>\n${transcript}\n</transcript>`
        : transcript;

      const abort = new AbortController();
      abortRef.current = abort;
      inFlightRef.current = true;
      state.setLiveInsightStatus("running");
      state.setLiveInsightError(null);
      log.info(
        `tick: calling LLM — model=${model} newSegments=${newSegments.length} totalSegments=${allSegments.length} transcriptChars=${transcript.length} previousChars=${previousContent?.length ?? 0}`,
        "insights",
      );

      try {
        const response = await client.chat.completions.create(
          {
            model,
            messages: [
              {
                role: "system",
                content: insight.prompt + OUTPUT_FORMAT_GUIDANCE,
              },
              { role: "user", content: userMessage },
            ],
          },
          { signal: abort.signal },
        );
        if (abort.signal.aborted) {
          log.debug("tick: aborted before response landed", "insights");
          return;
        }
        const rawContent = response.choices[0]?.message?.content ?? "";
        const content = rawContent.trim();
        log.info(
          `tick: LLM result — chars=${content.length}${content.length === 0 ? " (empty)" : ""}`,
          "insights",
        );
        // Always advance the watermark + write a result, even when the
        // model returns nothing. Otherwise the overlay sits at "Waiting…"
        // forever when the prompt legitimately produces "no output" runs.
        // Advance the watermark to the FULL segment count, not the slice
        // length. The next tick should pick up from here — that's the
        // whole point of "send only new segments since the last run."
        processedCountRef.current = allSegments.length;
        state.setLiveInsightResult({
          insightId: insight.id,
          content: content.length > 0 ? content : EMPTY_RESULT_PLACEHOLDER,
          generatedAt: new Date().toISOString(),
          segmentCountAtRun: allSegments.length,
        });
        state.setLiveInsightStatus("idle");
      } catch (e) {
        if (abort.signal.aborted) return;
        const msg = e instanceof Error ? e.message : String(e);
        log.warn(`tick: LLM call failed — ${msg}`, "insights");
        state.setLiveInsightStatus("error");
        state.setLiveInsightError(msg);
      } finally {
        inFlightRef.current = false;
        if (abortRef.current === abort) abortRef.current = null;
      }
    }

    // Fire one tick immediately so a freshly-set Active Insight produces
    // output without waiting for the first interval (the backfill case).
    void runTick();
    const intervalId = window.setInterval(runTick, heartbeatMs);

    return () => {
      window.clearInterval(intervalId);
      abortRef.current?.abort();
      abortRef.current = null;
      inFlightRef.current = false;
      log.debug("engine: torn down", "insights");
    };
  }, [
    enabled,
    liveTranscriptionActive,
    currentInsightId,
    currentInsightExists,
    currentEnabled,
    currentHeartbeatSeconds,
  ]);
}

/** Cap the previous-result payload, preserving the most recent characters.
 *  For short rolling summaries this is a no-op; for any future append-style
 *  insight whose previous content can grow indefinitely, we trim the head
 *  rather than the tail so the model sees the latest list state. */
function truncatePrevious(content: string): string {
  if (content.length <= PREVIOUS_CONTEXT_MAX_CHARS) return content;
  return "…" + content.slice(-(PREVIOUS_CONTEXT_MAX_CHARS - 1));
}
