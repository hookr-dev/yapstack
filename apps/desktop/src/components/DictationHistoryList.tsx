import { useEffect, useMemo, useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { DictationFeedEntry } from "@/components/DictationFeedEntry";
import { ArrowLeft, Mic, Trash2 } from "lucide-react";
import type { DbDictationHistory } from "@/lib/db";

function groupByDay(entries: DbDictationHistory[]): { label: string; entries: DbDictationHistory[] }[] {
  const groups: Map<string, DbDictationHistory[]> = new Map();

  for (const entry of entries) {
    const date = new Date(entry.created_at + "Z");
    const today = new Date();
    const yesterday = new Date();
    yesterday.setDate(yesterday.getDate() - 1);

    let label: string;
    if (date.toDateString() === today.toDateString()) {
      label = "Today";
    } else if (date.toDateString() === yesterday.toDateString()) {
      label = "Yesterday";
    } else {
      label = date.toLocaleDateString(undefined, {
        weekday: "long",
        month: "short",
        day: "numeric",
      });
    }

    if (!groups.has(label)) groups.set(label, []);
    groups.get(label)!.push(entry);
  }

  return Array.from(groups.entries()).map(([label, entries]) => ({
    label,
    entries,
  }));
}

export function DictationHistoryList() {
  const history = useAppStore((s) => s.dictationHistory);
  const loadDictationHistory = useAppStore((s) => s.loadDictationHistory);
  const clearDictationHistory = useAppStore((s) => s.clearDictationHistory);
  const setListFilter = useAppStore((s) => s.setListFilter);
  const [clearDialogOpen, setClearDialogOpen] = useState(false);
  const [highlightId, setHighlightId] = useState<string | null>(null);

  useEffect(() => {
    loadDictationHistory();
  }, [loadDictationHistory]);

  // Handle scroll-to-entry requests from the Cmd+K search. The event may arrive
  // before `loadDictationHistory()` has resolved, so retry briefly until the
  // entry lands in the DOM.
  useEffect(() => {
    function handler(e: Event) {
      const detail = (e as CustomEvent<{ dictationId: string }>).detail;
      if (!detail?.dictationId) return;
      const id = detail.dictationId;
      let attempts = 0;
      const tryScroll = () => {
        const el = document.querySelector<HTMLElement>(
          `[data-dictation-id="${CSS.escape(id)}"]`,
        );
        if (el) {
          el.scrollIntoView({ behavior: "smooth", block: "center" });
          setHighlightId(id);
          window.setTimeout(() => setHighlightId(null), 1600);
          return;
        }
        if (attempts++ < 20) window.setTimeout(tryScroll, 50);
      };
      tryScroll();
    }
    window.addEventListener("yapstack:scroll-to-dictation", handler);
    return () =>
      window.removeEventListener("yapstack:scroll-to-dictation", handler);
  }, []);

  const grouped = useMemo(() => groupByDay(history), [history]);

  const header = (
    <div className="flex items-center gap-2 border-b px-4 py-2 shrink-0">
      <button
        className="rounded-md p-1 text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
        onClick={() => setListFilter({ type: "all" })}
        aria-label="Back to All Notes"
      >
        <ArrowLeft className="h-4 w-4" />
      </button>
      <Mic className="h-4 w-4 text-muted-foreground" />
      <span className="text-sm font-medium">Dictation History</span>
      <div className="flex-1" />
      {history.length > 0 && (
        <Button
          variant="inline-destructive"
          size="inline"
          onClick={() => setClearDialogOpen(true)}
        >
          <Trash2 className="h-3 w-3" />
          Clear All
        </Button>
      )}
    </div>
  );

  if (history.length === 0) {
    return (
      <div className="flex flex-1 flex-col min-h-0">
        {header}
        <div className="flex flex-1 flex-col items-center justify-center gap-4 p-8 pb-20">
          <Mic className="h-8 w-8 text-muted-foreground/40" />
          <p className="text-center text-sm text-muted-foreground">
            No dictation history yet
          </p>
          <p className="text-center text-xs text-muted-foreground/60">
            Use a dictation keybind to start recording
          </p>
        </div>
      </div>
    );
  }

  const clearDialog = (
    <AlertDialog open={clearDialogOpen} onOpenChange={setClearDialogOpen}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Clear dictation history?</AlertDialogTitle>
          <AlertDialogDescription>
            This will permanently delete all dictation history entries. This action cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-white hover:bg-destructive/90"
            onClick={() => clearDictationHistory()}
          >
            Clear All
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );

  return (
    <div className="flex flex-1 flex-col min-h-0">
      {header}
      <ScrollArea className="min-h-0 flex-1">
        <div className="pb-28">
          {grouped.map((group) => (
            <div key={group.label}>
              <div className="sticky top-0 z-10 bg-background/80 backdrop-blur-sm px-4 py-1.5 border-b border-border/20">
                <span className="text-[11px] font-medium text-muted-foreground">
                  {group.label}
                </span>
              </div>
              {group.entries.map((entry) => (
                <DictationFeedEntry
                  key={entry.id}
                  entry={entry}
                  highlighted={highlightId === entry.id}
                />
              ))}
            </div>
          ))}
        </div>
      </ScrollArea>
      {clearDialog}
    </div>
  );
}
