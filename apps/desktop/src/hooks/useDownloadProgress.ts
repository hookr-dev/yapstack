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
      setModelDownloadProgress(payload.percent);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setModelDownloadProgress]);
}
