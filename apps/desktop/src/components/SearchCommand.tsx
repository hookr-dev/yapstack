import { useState, useEffect, useCallback, useMemo, type ReactNode } from "react";
import { useAppStore } from "@/stores/appStore";
import {
  CommandDialog,
  CommandInput,
  CommandList,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandSeparator,
  CommandShortcut,
  CommandFooter,
  CommandLoading,
} from "@/components/ui/command";
import {
  searchSegments,
  searchNotes,
  searchSessionsByTitle,
  searchFolders,
  searchDictations,
} from "@/lib/db";
import type { SearchResult, DictationSearchResult } from "@/lib/db";
import { trackSearchUsed } from "@/lib/analytics";
import { formatShortcutDisplay } from "@/lib/utils";
import { getBinding } from "@/lib/shortcuts";
import {
  Mic,
  PenLine,
  FileText,
  Inbox,
  Pin,
  Folder,
  Settings,
  Plus,
  FolderPlus,
  Search,
  ArrowUp,
  ArrowDown,
  CornerDownLeft,
} from "lucide-react";
import { ICON_MAP } from "@/lib/folder-constants";

function TypeBadge({ type }: { type: "session" | "note" | "segment" | "folder" | "dictation" }) {
  const styles: Record<string, string> = {
    session: "bg-blue-500/15 text-blue-600 dark:text-blue-400",
    note: "bg-green-500/15 text-green-600 dark:text-green-400",
    segment: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
    folder: "bg-purple-500/15 text-purple-600 dark:text-purple-400",
    dictation: "bg-pink-500/15 text-pink-600 dark:text-pink-400",
  };
  const labels: Record<string, string> = {
    session: "Session",
    note: "Note",
    segment: "Transcript",
    folder: "Folder",
    dictation: "Dictation",
  };
  return (
    <span className={`ml-auto shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium ${styles[type]}`}>
      {labels[type]}
    </span>
  );
}

function FooterKey({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex h-5 min-w-5 items-center justify-center rounded border border-border/50 bg-muted/50 px-1 text-[10px] font-medium keybind-display">
      {children}
    </kbd>
  );
}

function highlightSnippet(text: string, query: string): ReactNode {
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  const idx = lower.indexOf(q);
  if (idx === -1) return text.substring(0, 100);

  const start = Math.max(0, idx - 40);
  const end = Math.min(text.length, idx + query.length + 40);

  const before = (start > 0 ? "..." : "") + text.substring(start, idx);
  const match = text.substring(idx, idx + query.length);
  const after = text.substring(idx + query.length, end) + (end < text.length ? "..." : "");

  return (
    <>
      {before}
      <mark className="command-highlight">{match}</mark>
      {after}
    </>
  );
}

export function SearchCommand({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const openSession = useAppStore((s) => s.openSession);
  const navigateTo = useAppStore((s) => s.navigateTo);
  const setListFilter = useAppStore((s) => s.setListFilter);
  const folders = useAppStore((s) => s.folders);
  const sessions = useAppStore((s) => s.sessions);
  const createManualNote = useAppStore((s) => s.createManualNote);
  const selectedSessionId = useAppStore((s) => s.selectedSessionId);
  const currentView = useAppStore((s) => s.currentView);
  const togglePin = useAppStore((s) => s.togglePin);
  const shortcutBindings = useAppStore((s) => s.settings.shortcutBindings);

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [folderResults, setFolderResults] = useState<{ id: string; name: string }[]>([]);
  const [dictationResults, setDictationResults] = useState<DictationSearchResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [newFolderRequested, setNewFolderRequested] = useState(false);

  // Recent sessions (last 5 opened/created)
  const recentSessions = useMemo(
    () => sessions.slice(0, 5),
    [sessions],
  );

  // Helper for shortcut display
  const shortcut = (id: string) => formatShortcutDisplay(getBinding(id, shortcutBindings));

  // Debounced search
  useEffect(() => {
    if (!query.trim()) {
      setResults([]);
      setFolderResults([]);
      setDictationResults([]);
      return;
    }

    setSearching(true);
    const timeout = setTimeout(async () => {
      try {
        const [segResults, noteResults, titleResults, fResults, dResults] = await Promise.all([
          searchSegments(query.trim()),
          searchNotes(query.trim()),
          searchSessionsByTitle(query.trim()),
          searchFolders(query.trim()),
          searchDictations(query.trim()),
        ]);
        setResults([...titleResults, ...noteResults, ...segResults]);
        setFolderResults(fResults);
        setDictationResults(dResults);
        trackSearchUsed();
      } catch (e) {
        console.error("Search failed:", e);
      } finally {
        setSearching(false);
      }
    }, 300);

    return () => clearTimeout(timeout);
  }, [query]);

  const handleSelect = useCallback(
    (sessionId: string) => {
      openSession(sessionId);
      onOpenChange(false);
      setQuery("");
    },
    [openSession, onOpenChange],
  );

  const handleAction = useCallback(
    (action: () => void) => {
      action();
      onOpenChange(false);
      setQuery("");
    },
    [onOpenChange],
  );

  const handleSelectDictation = useCallback(
    (dictationId: string) => {
      setListFilter({ type: "dictation" });
      navigateTo("note-list");
      onOpenChange(false);
      setQuery("");
      // List may still be mounting or loading entries — fire the event after
      // the filter switch has a chance to render.
      requestAnimationFrame(() => {
        window.dispatchEvent(
          new CustomEvent("yapstack:scroll-to-dictation", {
            detail: { dictationId },
          }),
        );
      });
    },
    [setListFilter, navigateTo, onOpenChange],
  );

  // Reset on close
  useEffect(() => {
    if (!open) {
      setQuery("");
      setResults([]);
      setFolderResults([]);
      setDictationResults([]);
      setNewFolderRequested(false);
    }
  }, [open]);

  // Emit folder dialog request
  useEffect(() => {
    if (newFolderRequested) {
      window.dispatchEvent(new CustomEvent("yapstack:open-new-folder-dialog"));
      setNewFolderRequested(false);
    }
  }, [newFolderRequested]);

  const hasQuery = query.trim().length > 0;
  const sessionResults = results.filter((r) => r.type === "session");
  const noteResults = results.filter((r) => r.type === "note");
  const segmentResults = results.filter((r) => r.type === "segment");

  // Filter commands by query for non-empty queries
  const matchesQuery = (label: string) =>
    !hasQuery || label.toLowerCase().includes(query.toLowerCase());

  const canTogglePin = currentView === "note-detail" && selectedSessionId;

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange} shouldFilter={false}>
      <div className="relative">
        {searching && <CommandLoading />}
        <CommandInput
          placeholder="Search or type a command..."
          value={query}
          onValueChange={setQuery}
        />
      </div>
      <CommandList>
        {/* Empty state when query typed but no results */}
        {hasQuery && !searching && results.length === 0 && folderResults.length === 0 && dictationResults.length === 0 && (
          <CommandEmpty>
            <div className="flex flex-col items-center gap-2">
              <Search className="h-10 w-10 text-muted-foreground/30" />
              <p className="text-sm text-muted-foreground">No results for &ldquo;{query}&rdquo;</p>
              <p className="text-xs text-muted-foreground/60">Try a different search term</p>
            </div>
          </CommandEmpty>
        )}

        {/* Search results when query is present */}
        {hasQuery && (
          <>
            {folderResults.length > 0 && (
              <CommandGroup heading="Folders">
                {folderResults.map((f) => {
                  const FolderIcon = ICON_MAP[folders.find((fl) => fl.id === f.id)?.icon ?? ""] ?? Folder;
                  const folderColor = folders.find((fl) => fl.id === f.id)?.color;
                  return (
                    <CommandItem
                      key={`folder-${f.id}`}
                      value={`folder-${f.id}`}
                      onSelect={() =>
                        handleAction(() => {
                          setListFilter({ type: "folder", folderId: f.id });
                          navigateTo("note-list");
                        })
                      }
                    >
                      <FolderIcon
                        className="shrink-0 text-muted-foreground"
                        style={folderColor ? { color: folderColor } : undefined}
                      />
                      <span className="text-sm font-medium truncate">{f.name}</span>
                      <TypeBadge type="folder" />
                    </CommandItem>
                  );
                })}
              </CommandGroup>
            )}
            {sessionResults.length > 0 && (
              <CommandGroup heading="Sessions">
                {sessionResults.map((result) => (
                  <CommandItem
                    key={`session-${result.sessionId}`}
                    value={`session-${result.sessionId}`}
                    onSelect={() => handleSelect(result.sessionId)}
                  >
                    <FileText className="shrink-0 text-muted-foreground" />
                    <span className="text-sm font-medium truncate">
                      {result.sessionTitle}
                    </span>
                    <TypeBadge type="session" />
                  </CommandItem>
                ))}
              </CommandGroup>
            )}
            {noteResults.length > 0 && (
              <CommandGroup heading="Notes">
                {noteResults.map((result) => (
                  <CommandItem
                    key={`note-${result.sessionId}`}
                    value={`note-${result.sessionId}`}
                    onSelect={() => handleSelect(result.sessionId)}
                  >
                    <PenLine className="shrink-0 text-muted-foreground" />
                    <div className="flex flex-col gap-0.5 min-w-0">
                      <span className="text-sm font-medium truncate">
                        {result.sessionTitle}
                      </span>
                      <span className="text-xs text-muted-foreground truncate">
                        {highlightSnippet(result.snippet, query)}
                      </span>
                    </div>
                    <TypeBadge type="note" />
                  </CommandItem>
                ))}
              </CommandGroup>
            )}
            {segmentResults.length > 0 && (
              <CommandGroup heading="Transcripts">
                {segmentResults.map((result, i) => (
                  <CommandItem
                    key={`seg-${result.sessionId}-${i}`}
                    value={`seg-${result.sessionId}-${i}`}
                    onSelect={() => handleSelect(result.sessionId)}
                  >
                    <Mic className="shrink-0 text-muted-foreground" />
                    <div className="flex flex-col gap-0.5 min-w-0">
                      <span className="text-sm font-medium truncate">
                        {result.sessionTitle}
                      </span>
                      <span className="text-xs text-muted-foreground truncate">
                        {highlightSnippet(result.snippet, query)}
                      </span>
                    </div>
                    <TypeBadge type="segment" />
                  </CommandItem>
                ))}
              </CommandGroup>
            )}
            {dictationResults.length > 0 && (
              <CommandGroup heading="Dictations">
                {dictationResults.map((result) => (
                  <CommandItem
                    key={`dict-${result.dictationId}`}
                    value={`dict-${result.dictationId}`}
                    onSelect={() => handleSelectDictation(result.dictationId)}
                  >
                    <Mic className="shrink-0 text-muted-foreground" />
                    <div className="flex flex-col gap-0.5 min-w-0">
                      <span className="text-sm font-medium truncate">
                        {result.slotName}
                      </span>
                      <span className="text-xs text-muted-foreground truncate">
                        {highlightSnippet(result.snippet, query)}
                      </span>
                    </div>
                    <TypeBadge type="dictation" />
                  </CommandItem>
                ))}
              </CommandGroup>
            )}
          </>
        )}

        {/* Command palette when query is empty */}
        {!hasQuery && (
          <>
            {/* Navigation */}
            <CommandGroup heading="Navigation">
              {matchesQuery("All Sessions") && (
                <CommandItem
                  value="nav-all"
                  onSelect={() =>
                    handleAction(() => {
                      setListFilter({ type: "all" });
                      navigateTo("note-list");
                    })
                  }
                >
                  <Inbox className="shrink-0 text-muted-foreground" />
                  <span className="text-sm">All Sessions</span>
                  <CommandShortcut>{shortcut("filter-all")}</CommandShortcut>
                </CommandItem>
              )}
              {matchesQuery("Pinned") && (
                <CommandItem
                  value="nav-pinned"
                  onSelect={() =>
                    handleAction(() => {
                      setListFilter({ type: "pinned" });
                      navigateTo("note-list");
                    })
                  }
                >
                  <Pin className="shrink-0 text-muted-foreground" />
                  <span className="text-sm">Pinned</span>
                  <CommandShortcut>{shortcut("filter-pinned")}</CommandShortcut>
                </CommandItem>
              )}
              {folders.map((f) => {
                if (!matchesQuery(f.name)) return null;
                const FolderIcon = f.icon ? (ICON_MAP[f.icon] ?? Folder) : Folder;
                return (
                  <CommandItem
                    key={`nav-folder-${f.id}`}
                    value={`nav-folder-${f.id}`}
                    onSelect={() =>
                      handleAction(() => {
                        setListFilter({ type: "folder", folderId: f.id });
                        navigateTo("note-list");
                      })
                    }
                  >
                    <FolderIcon
                      className="shrink-0"
                      style={f.color ? { color: f.color } : undefined}
                    />
                    <span className="text-sm">{f.name}</span>
                  </CommandItem>
                );
              })}
              {matchesQuery("Settings") && (
                <CommandItem
                  value="nav-settings"
                  onSelect={() => handleAction(() => navigateTo("settings"))}
                >
                  <Settings className="shrink-0 text-muted-foreground" />
                  <span className="text-sm">Settings</span>
                  <CommandShortcut>{shortcut("open-settings")}</CommandShortcut>
                </CommandItem>
              )}
            </CommandGroup>

            <CommandSeparator />

            {/* Actions */}
            <CommandGroup heading="Actions">
              {matchesQuery("New Session") && (
                <CommandItem
                  value="action-new-session"
                  onSelect={() =>
                    handleAction(() => {
                      window.dispatchEvent(new CustomEvent("yapstack:create-session"));
                    })
                  }
                >
                  <Plus className="shrink-0 text-muted-foreground" />
                  <span className="text-sm">New Session</span>
                </CommandItem>
              )}
              {matchesQuery("New Note") && (
                <CommandItem
                  value="action-new-note"
                  onSelect={() => handleAction(() => createManualNote())}
                >
                  <PenLine className="shrink-0 text-muted-foreground" />
                  <span className="text-sm">New Note</span>
                  <CommandShortcut>{shortcut("new-note")}</CommandShortcut>
                </CommandItem>
              )}
              {matchesQuery("New Folder") && (
                <CommandItem
                  value="action-new-folder"
                  onSelect={() =>
                    handleAction(() => setNewFolderRequested(true))
                  }
                >
                  <FolderPlus className="shrink-0 text-muted-foreground" />
                  <span className="text-sm">New Folder</span>
                </CommandItem>
              )}
              {canTogglePin && matchesQuery("Toggle Pin") && (
                <CommandItem
                  value="action-toggle-pin"
                  onSelect={() =>
                    handleAction(() => togglePin(selectedSessionId!))
                  }
                >
                  <Pin className="shrink-0 text-muted-foreground" />
                  <span className="text-sm">Toggle Pin</span>
                  <CommandShortcut>{shortcut("pin-session")}</CommandShortcut>
                </CommandItem>
              )}
            </CommandGroup>

            {/* Recent */}
            {recentSessions.length > 0 && (
              <>
                <CommandSeparator />
                <CommandGroup heading="Recent">
                  {recentSessions.map((s) => (
                    <CommandItem
                      key={`recent-${s.id}`}
                      value={`recent-${s.id}`}
                      onSelect={() => handleSelect(s.id)}
                    >
                      {s.session_type === "manual" ? (
                        <PenLine className="shrink-0 text-muted-foreground" />
                      ) : (
                        <Mic className="shrink-0 text-muted-foreground" />
                      )}
                      <span className="text-sm truncate">
                        {s.title || "Untitled"}
                      </span>
                    </CommandItem>
                  ))}
                </CommandGroup>
              </>
            )}
          </>
        )}
      </CommandList>

      {/* Footer navigation hints */}
      <CommandFooter>
        <span className="inline-flex items-center gap-1.5">
          <FooterKey><ArrowUp className="!size-3" /></FooterKey>
          <FooterKey><ArrowDown className="!size-3" /></FooterKey>
          Navigate
        </span>
        <span className="inline-flex items-center gap-1.5">
          <FooterKey><CornerDownLeft className="!size-3" /></FooterKey>
          Open
        </span>
        <span className="inline-flex items-center gap-1.5">
          <FooterKey>Esc</FooterKey>
          Close
        </span>
      </CommandFooter>
    </CommandDialog>
  );
}
