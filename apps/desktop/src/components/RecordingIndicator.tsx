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

  return (
    <div
      className="flex h-screen w-screen items-center justify-center bg-transparent"
      data-tauri-drag-region
    >
      <div
        className="flex flex-col items-center gap-1 rounded-full bg-background pt-2 pb-1 px-2 shadow-2xl border border-white/[0.08]"
        data-tauri-drag-region
      >
        <button
          className="flex items-center justify-center rounded-full p-1 ring-2 ring-red-500 shadow-[0_0_12px_rgba(239,68,68,0.5)] animate-pulse cursor-pointer"
          onClick={() => emitEvent(EVENTS.RECORDING_INDICATOR_OPEN_MAIN)}
        >
          <YapStackIcon className="h-3 w-3 text-red-500" />
        </button>
        <div className="flex items-center justify-center py-1" data-tauri-drag-region>
          <GripVertical className="h-4 w-4 text-muted-foreground/40" data-tauri-drag-region />
        </div>
      </div>
    </div>
  );
}
