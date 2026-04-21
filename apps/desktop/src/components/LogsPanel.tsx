import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Copy, FolderOpen, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  clearLogs,
  formatLogEntries,
  getRecentLogs,
  revealLogDir,
  subscribeLogs,
  type LogEntry,
} from "@/lib/logs";

// Cap for the tailed live stream (the initial snapshot is already
// bounded by the backend ring buffer, but incoming `log://entry`
// events would grow without bound otherwise).
const CLIENT_CAP = 500;
const AUTO_SCROLL_THRESHOLD_PX = 40;

type LevelStyle = {
  /** Row background (errors/warnings tint; others stay transparent). */
  row: string;
  /** Color of the `[LEVEL]` bracket text. */
  level: string;
  /** Color of the message body (mutes DEBUG/TRACE). */
  message: string;
};

const LEVEL_STYLES: Record<LogEntry["level"], LevelStyle> = {
  ERROR: {
    row: "bg-red-500/10",
    level: "text-red-500",
    message: "text-red-500/90",
  },
  WARN: {
    row: "bg-yellow-500/[0.07]",
    level: "text-yellow-500",
    message: "text-yellow-200",
  },
  INFO: {
    row: "",
    level: "text-sky-400",
    message: "text-foreground",
  },
  DEBUG: {
    row: "",
    level: "text-muted-foreground",
    message: "text-muted-foreground",
  },
  TRACE: {
    row: "",
    level: "text-muted-foreground/60",
    message: "text-muted-foreground/60",
  },
};

function formatTs(ts: number): string {
  const d = new Date(ts);
  const h = d.getHours().toString().padStart(2, "0");
  const m = d.getMinutes().toString().padStart(2, "0");
  const s = d.getSeconds().toString().padStart(2, "0");
  return `${h}:${m}:${s}`;
}

/** Drop the `yapstack_` prefix (most targets share it) to save horizontal space. */
function shortTarget(target: string): string {
  return target.startsWith("yapstack_") ? target.slice("yapstack_".length) : target;
}

/**
 * One log row. Prefix fields are inline so wrapped lines of a long message
 * flow under the timestamp instead of being indented by fixed-width columns.
 * Click / Enter / Space toggles expand.
 */
function LogRow({ entry }: { entry: LogEntry }) {
  const [expanded, setExpanded] = useState(false);
  const style = LEVEL_STYLES[entry.level];
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => setExpanded((v) => !v)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          setExpanded((v) => !v);
        }
      }}
      className={`group cursor-pointer rounded px-1 py-0.5 hover:bg-muted/40 ${style.row} ${
        expanded ? "whitespace-pre-wrap break-all" : "line-clamp-3 break-words"
      }`}
      title={`${entry.target} · ${entry.level} · click to ${expanded ? "collapse" : "expand"}`}
    >
      <span className="text-muted-foreground/60 tabular-nums">
        [{formatTs(entry.ts_ms)}]
      </span>
      <span className={`font-medium ${style.level}`}>[{entry.level}]</span>
      <span className="text-muted-foreground/70">
        [{shortTarget(entry.target)}]
      </span>{" "}
      <span className={style.message}>{entry.message}</span>
    </div>
  );
}

export function LogsPanel() {
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const pinnedToBottom = useRef(true);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    (async () => {
      try {
        const snapshot = await getRecentLogs(200);
        if (cancelled) return;
        setEntries(snapshot);
      } catch (err) {
        if (!cancelled) console.error("failed to load recent logs", err);
      }

      unlisten = await subscribeLogs((entry) => {
        setEntries((prev) => {
          const next = prev.concat(entry);
          return next.length > CLIENT_CAP ? next.slice(next.length - CLIENT_CAP) : next;
        });
      });
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Tail: when pinned to bottom, jump to the end on every entries change
  // (including the initial snapshot load). useLayoutEffect so the scroll
  // happens before paint — avoids a brief flicker at the top on first mount.
  useLayoutEffect(() => {
    const vp = viewportRef.current;
    if (!vp || !pinnedToBottom.current) return;
    vp.scrollTop = vp.scrollHeight;
  }, [entries]);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const t = e.currentTarget;
    // "At the bottom" = within AUTO_SCROLL_THRESHOLD_PX of the end. Below
    // that, we treat the user as reading scroll-back and stop tailing;
    // scroll back down and tailing resumes automatically.
    pinnedToBottom.current =
      t.scrollHeight - t.clientHeight - t.scrollTop < AUTO_SCROLL_THRESHOLD_PX;
  };

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(formatLogEntries(entries));
      toast.success(`Copied ${entries.length} log ${entries.length === 1 ? "line" : "lines"}`);
    } catch (err) {
      toast.error("Failed to copy logs");
      console.error(err);
    }
  };

  const onReveal = async () => {
    try {
      await revealLogDir();
    } catch (err) {
      toast.error("Failed to reveal log folder");
      console.error(err);
    }
  };

  const onClear = async () => {
    try {
      await clearLogs();
      setEntries([]);
    } catch (err) {
      toast.error("Failed to clear logs");
      console.error(err);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div
        ref={viewportRef}
        onScroll={onScroll}
        className="h-[260px] divide-y divide-border/40 overflow-y-auto border-t border-border px-1 py-1 font-mono text-[10px] leading-4"
      >
        {entries.length === 0 ? (
          <div className="text-muted-foreground/60">No log entries yet.</div>
        ) : (
          entries.map((e, i) => (
            <LogRow key={`${e.ts_ms}-${i}`} entry={e} />
          ))
        )}
      </div>
      <div className="flex items-center justify-between gap-2 border-t border-border px-3 py-2">
        <span className="text-[10px] text-muted-foreground">
          {entries.length} {entries.length === 1 ? "entry" : "entries"}
        </span>
        <div className="flex items-center gap-1">
          <Button
            size="sm"
            variant="ghost"
            className="h-7 gap-1.5 px-2 text-[11px]"
            onClick={onCopy}
            disabled={entries.length === 0}
          >
            <Copy className="h-3 w-3" />
            Copy
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-7 gap-1.5 px-2 text-[11px]"
            onClick={onReveal}
          >
            <FolderOpen className="h-3 w-3" />
            Reveal
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-7 gap-1.5 px-2 text-[11px]"
            onClick={onClear}
            disabled={entries.length === 0}
          >
            <Trash2 className="h-3 w-3" />
            Clear
          </Button>
        </div>
      </div>
    </div>
  );
}
