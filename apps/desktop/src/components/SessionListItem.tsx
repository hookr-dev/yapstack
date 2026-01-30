import { useAppStore } from "@/stores/appStore";
import type { DbSession } from "@/lib/db";
import { cn } from "@/lib/utils";

function formatRelativeTime(dateStr: string): string {
  const date = new Date(dateStr + "Z");
  const diff = Date.now() - date.getTime();
  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return "Just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function SessionListItem({ session }: { session: DbSession }) {
  const selectedSessionId = useAppStore((s) => s.selectedSessionId);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const openSession = useAppStore((s) => s.openSession);

  const isSelected = selectedSessionId === session.id;
  const isRecording =
    session.id === activeSessionId || session.status === "recording";

  return (
    <button
      className={cn(
        "flex w-full flex-col gap-0.5 rounded-lg px-3 py-2 text-left transition-colors",
        isSelected ? "bg-accent" : "hover:bg-muted/50",
      )}
      onClick={() => openSession(session.id)}
    >
      <div className="flex min-w-0 items-center gap-2">
        {isRecording && (
          <>
            <span className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-red-500" aria-hidden />
            <span className="sr-only">Recording</span>
          </>
        )}
        <span className="truncate text-sm font-medium">
          {session.title || "Untitled Session"}
        </span>
      </div>
      <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
        <span>{formatRelativeTime(session.created_at)}</span>
        {session.total_segments > 0 && (
          <span>&middot; {session.total_segments} segments</span>
        )}
      </div>
    </button>
  );
}
