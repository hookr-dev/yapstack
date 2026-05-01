import { useEffect } from "react";
import { useAppStore } from "@/stores/appStore";
import { EVENTS, listenEvent } from "@/lib/events";

/** Listens for model download progress events and updates the store. Mounted in AppLayout. */
export function useDownloadProgress() {
  const setModelDownloadProgress = useAppStore(
    (s) => s.setModelDownloadProgress,
  );

  useEffect(() => {
    const unlisten = listenEvent(EVENTS.MODEL_DOWNLOAD_PROGRESS, (payload) => {
      // Backend emits percent as a [0.0, 1.0] ratio; UI consumers (Progress
      // component, TitleBar text) all expect 0–100.
      const pct = Math.min(100, Math.max(0, Math.round(payload.percent * 100)));
      // Monotonic within an active download: a stale or out-of-order event
      // can't pull the bar backward. New downloads reset progress to 0 via
      // direct store writes (appStore.ts), which bypasses this guard.
      const current = useAppStore.getState().modelDownloadProgress;
      if (current === null || pct > current) {
        setModelDownloadProgress(pct);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setModelDownloadProgress]);
}
