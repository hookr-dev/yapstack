import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
import { AppLayout } from "@/components/AppLayout";
import { useAppStore } from "@/stores/appStore";
import type { ThemeMode } from "@/stores/appStore";
import { useGlobalShortcuts } from "@/hooks/useGlobalShortcuts";
import { useDictation } from "@/hooks/useDictation";
import { useRecordingIndicator } from "@/hooks/useRecordingIndicator";
import { useUpdateCheck } from "@/hooks/useUpdateCheck";
import { DictationBubble } from "@/components/DictationBubble";
import { RecordingIndicator } from "@/components/RecordingIndicator";
import { WINDOWS } from "@/lib/events";

const WINDOW_POS_KEY = "yapstack-window-position";

interface WindowPosition {
  x: number;
  y: number;
  width: number;
  height: number;
}

function applyThemeClass(theme: ThemeMode) {
  const root = document.documentElement;
  if (theme === "dark") {
    root.classList.add("dark");
  } else if (theme === "light") {
    root.classList.remove("dark");
  } else {
    const prefersDark = window.matchMedia(
      "(prefers-color-scheme: dark)",
    ).matches;
    root.classList.toggle("dark", prefersDark);
  }
}

// Determine window type from URL params
const params = new URLSearchParams(window.location.search);
const windowType = params.get("window");

function MainApp() {
  const theme = useAppStore((s) => s.settings.theme);
  useGlobalShortcuts();
  useDictation();
  useRecordingIndicator();
  useUpdateCheck();

  // Apply theme on mount and when setting changes
  useEffect(() => {
    applyThemeClass(theme);

    if (theme === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      const handler = () => applyThemeClass("system");
      mq.addEventListener("change", handler);
      return () => mq.removeEventListener("change", handler);
    }
  }, [theme]);

  // Restore window position on mount, save on move/resize
  useEffect(() => {
    const appWindow = getCurrentWindow();

    // Restore saved position
    const saved = localStorage.getItem(WINDOW_POS_KEY);
    if (saved) {
      try {
        const pos: WindowPosition = JSON.parse(saved);
        appWindow.setPosition(new PhysicalPosition(pos.x, pos.y));
        appWindow.setSize(new PhysicalSize(pos.width, pos.height));
      } catch {
        // Ignore invalid saved position
      }
    }

    // Persist position/size on every move or resize so it survives Cmd+Q
    let saveTimer: ReturnType<typeof setTimeout>;
    const savePosition = () => {
      clearTimeout(saveTimer);
      saveTimer = setTimeout(async () => {
        try {
          const pos = await appWindow.outerPosition();
          const size = await appWindow.innerSize();
          localStorage.setItem(
            WINDOW_POS_KEY,
            JSON.stringify({ x: pos.x, y: pos.y, width: size.width, height: size.height }),
          );
        } catch {
          // Ignore errors
        }
      }, 500);
    };

    const unlistenMoved = appWindow.onMoved(savePosition);
    const unlistenResized = appWindow.onResized(savePosition);

    return () => {
      clearTimeout(saveTimer);
      unlistenMoved.then((fn) => fn());
      unlistenResized.then((fn) => fn());
    };
  }, []);

  // Close-to-minimize: hide window instead of destroying it
  useEffect(() => {
    const appWindow = getCurrentWindow();
    const unlisten = appWindow.onCloseRequested(async (event) => {
      event.preventDefault();
      await appWindow.hide();
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <div className="h-screen w-screen overflow-hidden bg-background text-foreground">
      <AppLayout />
    </div>
  );
}

function App() {
  if (windowType === WINDOWS.DICTATION) return <DictationBubble />;
  if (windowType === WINDOWS.RECORDING_INDICATOR) return <RecordingIndicator />;
  return <MainApp />;
}

export default App;
