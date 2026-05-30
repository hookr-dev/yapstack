import { useEffect, useState } from "react";
import { YapStackIcon } from "@/components/YapStackIcon";
import { GripVertical } from "lucide-react";
import { EVENTS, listenEvent, emitEvent } from "@/lib/events";
import { useOverlayStyles } from "@/hooks/useOverlayStyles";

export function RecordingIndicator() {
  const [active, setActive] = useState(false);
  useOverlayStyles();

  useEffect(() => {
    const unlisten = listenEvent(EVENTS.RECORDING_INDICATOR_ACTIVE, (payload) => {
      setActive(payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  if (!active) return null;

  // Dragging is Tauri's `data-tauri-drag-region`, which only starts a window
  // drag when the element *directly* under the pointer carries the attribute
  // (it does not walk up to ancestors). So every draggable surface is marked,
  // and the decorative icons get `pointer-events-none` so a press lands on the
  // drag-region behind them instead of the SVG. The button is deliberately not
  // a drag region — it stays a click target that opens the main window.
  return (
    <div
      className="flex h-screen w-screen items-center justify-center bg-transparent"
      data-tauri-drag-region
    >
      <div
        className="flex flex-col items-center gap-1 rounded-full bg-background pt-2 pb-1 px-2 shadow-lg border border-white/[0.08]"
        data-tauri-drag-region
      >
        <button
          className="flex items-center justify-center rounded-full p-1 ring-2 ring-red-500 shadow-[0_0_12px_rgba(239,68,68,0.5)] animate-pulse cursor-pointer"
          onClick={() => emitEvent(EVENTS.RECORDING_INDICATOR_OPEN_MAIN)}
        >
          <YapStackIcon className="pointer-events-none h-3 w-3 text-red-500" />
        </button>
        <div className="flex items-center justify-center py-1" data-tauri-drag-region>
          <GripVertical className="pointer-events-none h-4 w-4 text-muted-foreground/40" />
        </div>
      </div>
    </div>
  );
}
