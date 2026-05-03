import { useEffect } from "react";
import { toast } from "sonner";
import { useAppStore } from "@/stores/appStore";
import { commands } from "@/lib/tauri";
import { EVENTS, listenEvent } from "@/lib/events";

/** Listens for capture state changes and eagerly fetches status on mount. Mounted in AppLayout. */
export function useCaptureEvents() {
  const setCaptureStatus = useAppStore((s) => s.setCaptureStatus);
  const setBufferInfo = useAppStore((s) => s.setBufferInfo);
  const applyDeviceList = useAppStore((s) => s.applyDeviceList);

  useEffect(() => {
    const unlisten = Promise.all([
      listenEvent(EVENTS.CAPTURE_STATUS, (payload) => {
        const prev = useAppStore.getState().captureStatus;
        setCaptureStatus(payload);
        // Toast when capture transitions to Error (not on every poll)
        if (payload.state === "Error" && prev?.state !== "Error") {
          toast.error(payload.error_message ?? "Audio capture failed", {
            id: "capture-error",
          });
        }
      }),
      listenEvent(EVENTS.BUFFER_INFO, (payload) => {
        setBufferInfo(payload);
      }),
      // Hot-plug + default-device reaction. The device broker emits this
      // after a 250 ms debounce; we just apply the payload directly.
      listenEvent(EVENTS.DEVICES_CHANGED, (payload) => {
        applyDeviceList(payload);
      }),
    ]);

    // Eager fetch — closes the race window between mount and first event
    commands.getCaptureStatus().then((r) => {
      if (r.status === "ok") setCaptureStatus(r.data);
    });
    commands.getBufferInfo().then((r) => {
      if (r.status === "ok") setBufferInfo(r.data);
    });

    return () => {
      unlisten.then((fns) => fns.forEach((fn) => fn()));
    };
  }, [setCaptureStatus, setBufferInfo, applyDeviceList]);
}
