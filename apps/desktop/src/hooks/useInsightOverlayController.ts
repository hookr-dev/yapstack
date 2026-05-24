import { useEffect, useRef } from "react";
import { primaryMonitor } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import { useAppStore } from "@/stores/appStore";
import { commands } from "@/lib/tauri";
import { EVENTS, WINDOWS, listenEvent } from "@/lib/events";
import { shouldShowInsightOverlay } from "@/lib/insights";
import { log } from "@/lib/logger";

/**
 * Manages the floating Insight overlay window: show/hide based on enable +
 * live-session + active-insight state, and emit INSIGHT_STATE events to the
 * overlay so it can render the latest Insight result.
 *
 * Mounted in MainApp.
 */
export function useInsightOverlayController() {
  const enabled = useAppStore((s) => s.settings.insights.enabled);
  // The overlay tracks the Current Insight (runtime, ephemeral) — not the
  // Default (persisted). The Default seeds Current at session start; the
  // overlay's dropdown picks and × button only mutate Current.
  const currentInsightId = useAppStore((s) => s.currentInsightId);
  const liveTranscriptionActive = useAppStore((s) => s.liveTranscriptionActive);
  const liveInsightResult = useAppStore((s) => s.liveInsightResult);
  const liveInsightStatus = useAppStore((s) => s.liveInsightStatus);
  const liveInsightError = useAppStore((s) => s.liveInsightError);
  const setCurrentInsightId = useAppStore((s) => s.setCurrentInsightId);

  // Stable string key of the slot ids + names. Used as an emit-effect dep so
  // prompt-textarea keystrokes (which mutate the slots array) don't re-fire the
  // emit, while genuine list changes (rename / add / delete) do. Returning a
  // flat string lets Zustand's reference-equality skip identical updates.
  const slotsKey = useAppStore((s) =>
    s.settings.insights.slots
      .map((slot) => `${slot.id}:${slot.name}`)
      .join("|"),
  );

  const visibleRef = useRef(false);
  const positionedRef = useRef(false);

  // Listen for the overlay's close-button request and clear the Current
  // Insight for this session. The feature stays enabled and the Default
  // is untouched — next session starts fresh with the Default again.
  useEffect(() => {
    const unlisten = listenEvent(EVENTS.INSIGHT_HIDE_REQUEST, () => {
      setCurrentInsightId(null);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [setCurrentInsightId]);

  // Listen for the overlay's in-header Insight switcher — writes the new
  // Current Insight to runtime state only. Default in Settings is untouched.
  useEffect(() => {
    const unlisten = listenEvent(
      EVENTS.INSIGHT_CHANGE_ACTIVE,
      ({ insightId }) => {
        setCurrentInsightId(insightId);
      },
    );
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [setCurrentInsightId]);

  // Drive show/hide. The gate also requires the Current Insight to resolve to
  // an existing, enabled slot — otherwise a disabled Default at session start
  // or an Insight deleted/disabled mid-session would leave a zombie overlay the
  // engine never feeds. Returning a boolean keeps the selector stable across
  // prompt-textarea edits (the value only flips when visibility truly changes).
  const shouldShow = useAppStore((s) =>
    shouldShowInsightOverlay({
      enabled: s.settings.insights.enabled,
      liveTranscriptionActive: s.liveTranscriptionActive,
      currentInsightId: s.currentInsightId,
      slots: s.settings.insights.slots,
    }),
  );

  useEffect(() => {
    let cancelled = false;

    async function show() {
      try {
        const win = await WebviewWindow.getByLabel(WINDOWS.INSIGHT);
        if (!win || cancelled) return;
        // Position top-center on first show, similar to PRD intent.
        if (!positionedRef.current) {
          try {
            const monitor = await primaryMonitor();
            if (monitor) {
              const scale = monitor.scaleFactor;
              const screenW = monitor.size.width / scale;
              const monX = monitor.position.x / scale;
              const monY = monitor.position.y / scale;
              const winW = 480;
              const x = monX + (screenW - winW) / 2;
              // 56 px clears the macOS menu bar on standard displays and the
              // notched-MacBook menu strip (~37–44 px) with a small buffer.
              const y = monY + 56;
              await win.setPosition(new LogicalPosition(x, y));
            }
          } catch {
            // Positioning is best-effort.
          }
          positionedRef.current = true;
        }
        await commands.showOverlayPanel(WINDOWS.INSIGHT);
        visibleRef.current = true;
      } catch {
        // Insight window may not exist on platforms without an NSPanel build.
      }
    }

    async function hide() {
      visibleRef.current = false;
      try {
        await commands.hideOverlayPanel(WINDOWS.INSIGHT);
      } catch {
        // No-op when the window isn't available.
      }
    }

    log.debug(
      `overlay: gate=${shouldShow} (enabled=${enabled}, live=${liveTranscriptionActive}, currentId=${currentInsightId ?? "null"})`,
      "insights",
    );
    if (shouldShow) void show();
    else void hide();

    return () => {
      cancelled = true;
    };
  }, [shouldShow]);

  // Mirror runtime state into Tauri events. The overlay window has no
  // Zustand access — INSIGHT_STATE is the only channel.
  useEffect(() => {
    if (!visibleRef.current && !shouldShow) return;
    const state = useAppStore.getState();
    const insight = state.settings.insights.slots.find(
      (s) => s.id === state.currentInsightId,
    );
    const overlaySlots = state.settings.insights.slots.map((s) => ({
      id: s.id,
      name: s.name,
    }));
    void (async () => {
      try {
        const win = await WebviewWindow.getByLabel(WINDOWS.INSIGHT);
        if (!win) return;
        await win.emit(EVENTS.INSIGHT_STATE, {
          insightName: insight?.name ?? "Insight",
          status: liveInsightStatus,
          content: liveInsightResult?.content ?? null,
          generatedAt: liveInsightResult?.generatedAt ?? null,
          error: liveInsightError,
          currentInsightId: state.currentInsightId,
          slots: overlaySlots,
        });
        log.debug(
          `overlay: emitted state — status=${liveInsightStatus} hasContent=${!!liveInsightResult?.content} hasError=${!!liveInsightError}`,
          "insights",
        );
      } catch (e) {
        log.warn(
          `overlay: emit failed — ${e instanceof Error ? e.message : String(e)}`,
          "insights",
        );
      }
    })();
  }, [
    shouldShow,
    liveInsightResult,
    liveInsightStatus,
    liveInsightError,
    currentInsightId,
    slotsKey,
  ]);
}
