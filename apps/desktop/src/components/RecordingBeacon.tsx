import { useState, useEffect } from "react";
import { useAppStore } from "@/stores/appStore";
import { formatElapsed } from "@/lib/utils";

export function RecordingBeacon() {
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const activeSessionStartTime = useAppStore((s) => s.activeSessionStartTime);
  const sessionStopping = useAppStore((s) => s.sessionStopping);
  const openSession = useAppStore((s) => s.openSession);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    if (!activeSessionStartTime || sessionStopping) return;
    const update = () => setElapsed(Date.now() - activeSessionStartTime);
    update();
    const id = setInterval(update, 1000);
    return () => clearInterval(id);
  }, [activeSessionStartTime, sessionStopping]);

  if (!activeSessionId) return null;

  if (sessionStopping) {
    return (
      <button
        className="flex w-full items-center gap-3 rounded-lg bg-amber-500/10 px-3 py-2.5 text-left transition-colors hover:bg-amber-500/15 animate-pulse"
        onClick={() => openSession(activeSessionId)}
      >
        <span className="relative flex h-3 w-3 shrink-0">
          <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-amber-500 opacity-75" />
          <span className="relative inline-flex h-3 w-3 rounded-full bg-amber-500" />
        </span>
        <span className="truncate text-sm font-medium text-amber-600 dark:text-amber-400">
          Finalizing…
        </span>
      </button>
    );
  }

  return (
    <button
      className="flex w-full items-center gap-3 rounded-lg bg-destructive/10 px-3 py-2.5 text-left transition-colors hover:bg-destructive/15"
      onClick={() => openSession(activeSessionId)}
    >
      <span className="relative flex h-3 w-3 shrink-0">
        <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-destructive opacity-75" />
        <span className="relative inline-flex h-3 w-3 rounded-full bg-destructive" />
      </span>
      <div className="flex min-w-0 flex-1 items-center justify-between">
        <span className="truncate text-sm font-medium text-destructive">
          Recording
        </span>
        <span className="shrink-0 font-mono text-xs text-destructive/80">
          {formatElapsed(elapsed)}
        </span>
      </div>
    </button>
  );
}
