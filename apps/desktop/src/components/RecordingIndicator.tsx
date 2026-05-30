import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { YapStackIcon } from "@/components/YapStackIcon";
import { GripVertical } from "lucide-react";
import { EVENTS, listenEvent, emitEvent } from "@/lib/events";
import { useOverlayStyles } from "@/hooks/useOverlayStyles";

// Pointer must travel this many px before a press becomes a window drag.
// Below the threshold the press stays a click (opens the main window).
const DRAG_THRESHOLD_PX = 4;

export function RecordingIndicator() {
  const [active, setActive] = useState(false);
  useOverlayStyles();

  // Press origin; null whenever no left-button press is in flight. Cleared the
  // instant a drag starts so startDragging() fires exactly once per gesture.
  const pressOrigin = useRef<{ x: number; y: number } | null>(null);
  // True once a press crossed the threshold, so the trailing click is swallowed
  // and repositioning the pill never pops the main window open.
  const draggedRef = useRef(false);

  useEffect(() => {
    const unlisten = listenEvent(EVENTS.RECORDING_INDICATOR_ACTIVE, (payload) => {
      setActive(payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  if (!active) return null;

  const handlePointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    pressOrigin.current = { x: e.clientX, y: e.clientY };
    draggedRef.current = false;
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    const origin = pressOrigin.current;
    if (!origin) return;
    const dx = e.clientX - origin.x;
    const dy = e.clientY - origin.y;
    if (dx * dx + dy * dy < DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX) return;
    // Threshold crossed: hand off to the OS window drag and disarm so we don't
    // kick off a second drag mid-gesture. The native drag consumes the release,
    // so no pointerup/click follows — draggedRef is reset on the next press.
    pressOrigin.current = null;
    draggedRef.current = true;
    void getCurrentWindow().startDragging();
  };

  const endPress = () => {
    pressOrigin.current = null;
  };

  const handleOpen = () => {
    if (draggedRef.current) {
      draggedRef.current = false;
      return;
    }
    emitEvent(EVENTS.RECORDING_INDICATOR_OPEN_MAIN);
  };

  return (
    <div
      className="flex h-screen w-screen items-center justify-center bg-transparent"
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={endPress}
      onPointerCancel={endPress}
    >
      {/* No grab cursor: this is a non-activating overlay panel, so while the
          user is in another app macOS gives cursor control to that app and the
          webview never receives the hover events that would apply CSS `cursor`.
          Drag still works (mouse-down IS delivered); the grip glyph is the
          visible drag affordance instead. */}
      <div className="flex flex-col items-center gap-1 rounded-full bg-background pt-2 pb-1 px-2 shadow-lg border border-white/[0.08]">
        <button
          className="flex items-center justify-center rounded-full p-1 ring-2 ring-red-500 shadow-[0_0_12px_rgba(239,68,68,0.5)] animate-pulse"
          onClick={handleOpen}
        >
          <YapStackIcon className="pointer-events-none h-3 w-3 text-red-500" />
        </button>
        <div className="flex items-center justify-center py-1">
          <GripVertical className="pointer-events-none h-4 w-4 text-muted-foreground/40" />
        </div>
      </div>
    </div>
  );
}
