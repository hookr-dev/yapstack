import { useEffect, useRef } from "react";
import { getCurrentWindow, primaryMonitor } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import { useAppStore } from "@/stores/appStore";
import { commands } from "@/lib/tauri";
import { EVENTS, WINDOWS, listenEvent } from "@/lib/events";

/** Manages the floating recording indicator window visibility during active sessions. Mounted in App.tsx. */
export function useRecordingIndicator() {
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const showRecordingIndicator = useAppStore(
    (s) => s.settings.showRecordingIndicator,
  );
  const isRecording = !!activeSessionId;
  const visibleRef = useRef(false);
  const positionedRef = useRef(false);

  useEffect(() => {
    if (!showRecordingIndicator || !isRecording) {
      // Hide if setting disabled or not recording
      if (visibleRef.current) {
        visibleRef.current = false;
        WebviewWindow.getByLabel(WINDOWS.RECORDING_INDICATOR).then((win) => {
          if (win) win.emit(EVENTS.RECORDING_INDICATOR_ACTIVE, false);
        }).catch(() => {});
        void commands.hideOverlayPanel(WINDOWS.RECORDING_INDICATOR);
      }
      if (!isRecording) return;
    }

    const appWindow = getCurrentWindow();
    let cancelled = false;

    const show = async () => {
      try {
        const win = await WebviewWindow.getByLabel(WINDOWS.RECORDING_INDICATOR);
        if (!win || cancelled) return;
        // Position middle-right on first show
        if (!positionedRef.current) {
          try {
            const monitor = await primaryMonitor();
            if (monitor) {
              const scale = monitor.scaleFactor;
              const screenW = monitor.size.width / scale;
              const screenH = monitor.size.height / scale;
              const monX = monitor.position.x / scale;
              const monY = monitor.position.y / scale;
              const winW = 56;
              const winH = 120;
              const x = monX + screenW - winW - 12;
              const y = monY + (screenH - winH) / 2;
              await win.setPosition(new LogicalPosition(x, y));
            }
          } catch {
            // Ignore positioning errors
          }
          positionedRef.current = true;
        }
        await win.emit(EVENTS.RECORDING_INDICATOR_ACTIVE, true);
        await commands.showOverlayPanel(WINDOWS.RECORDING_INDICATOR);
        visibleRef.current = true;
      } catch {
        // Recording indicator window may not exist
      }
    };

    const hide = async () => {
      visibleRef.current = false;
      try {
        const win = await WebviewWindow.getByLabel(WINDOWS.RECORDING_INDICATOR);
        if (!win || cancelled) return;
        await win.emit(EVENTS.RECORDING_INDICATOR_ACTIVE, false);
        await commands.hideOverlayPanel(WINDOWS.RECORDING_INDICATOR);
      } catch {
        // Recording indicator window may not exist
      }
    };

    const unlisten = appWindow.onFocusChanged(({ payload: focused }) => {
      if (cancelled) return;
      if (!useAppStore.getState().settings.showRecordingIndicator) return;
      if (!useAppStore.getState().activeSessionId) return;
      if (focused) {
        void hide();
      } else {
        void show();
      }
    });

    // Click on recording icon → show + focus main window on active session
    const unlistenOpen = listenEvent(EVENTS.RECORDING_INDICATOR_OPEN_MAIN, () => {
      if (cancelled) return;
      const { activeSessionId, navigateTo } = useAppStore.getState();
      if (activeSessionId) {
        navigateTo("note-detail", activeSessionId);
      }
      appWindow.show();
      appWindow.setFocus();
    });

    // Show immediately if window is not focused
    appWindow.isFocused().then((focused) => {
      if (!cancelled && !focused && showRecordingIndicator && isRecording) {
        show();
      }
    });

    return () => {
      cancelled = true;
      unlisten.then((fn) => fn());
      unlistenOpen.then((fn) => fn());
      // Hide on cleanup
      if (visibleRef.current) {
        visibleRef.current = false;
        WebviewWindow.getByLabel(WINDOWS.RECORDING_INDICATOR).then((win) => {
          if (win) win.emit(EVENTS.RECORDING_INDICATOR_ACTIVE, false);
        }).catch(() => {});
        void commands.hideOverlayPanel(WINDOWS.RECORDING_INDICATOR);
      }
    };
  }, [isRecording, showRecordingIndicator]);
}
