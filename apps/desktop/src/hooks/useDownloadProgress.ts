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
      // component, TitleBar text, SetupBanner) all expect 0–100.
      setModelDownloadProgress(Math.round(payload.percent * 100));
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setModelDownloadProgress]);
}
