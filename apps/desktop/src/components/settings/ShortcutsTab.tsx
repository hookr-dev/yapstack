import { useState, useEffect, useCallback, useRef } from "react";
import { useAppStore } from "@/stores/appStore";
import {
  SHORTCUTS,
  SHORTCUT_CATEGORIES,
  findShortcutConflict,
  getBinding,
  eventToBinding,
  eventToGlobalBinding,
  shortcutCaptureActive,
} from "@/lib/shortcuts";
import { suspendGlobalShortcuts, resumeGlobalShortcuts } from "@/hooks/useGlobalShortcuts";
import type { ShortcutDefinition } from "@/lib/shortcuts";
import { formatGlobalShortcutDisplay, formatShortcutDisplay } from "@/lib/utils";
import { Globe, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { toast } from "sonner";

function ShortcutRow({
  shortcut,
  binding,
  onRebind,
  onReset,
  hasOverride,
  conflict,
}: {
  shortcut: ShortcutDefinition;
  binding: string;
  onRebind: (id: string) => void;
  onReset: (id: string) => void;
  hasOverride: boolean;
  conflict: string | null;
}) {
  const displayBinding = shortcut.isGlobal
    ? formatGlobalShortcutDisplay(binding)
    : formatShortcutDisplay(binding);

  return (
    <div className="flex items-center justify-between py-1.5 group">
      <div className="flex flex-col gap-0.5 min-w-0">
        <div className="flex items-center gap-1.5">
          <span className="text-xs font-medium">{shortcut.label}</span>
          {shortcut.isGlobal && (
            <span className="inline-flex items-center gap-0.5 rounded bg-muted px-1 py-0.5 text-[9px] text-muted-foreground">
              <Globe className="h-2.5 w-2.5" />
              Global
            </span>
          )}
        </div>
        <span className="text-[11px] text-muted-foreground">
          {shortcut.description}
        </span>
        {conflict && (
          <span className="text-[10px] text-destructive">
            Conflicts with: {conflict}
          </span>
        )}
      </div>
      <div className="flex items-center gap-1.5">
        {hasOverride && (
          <Button
            variant="inline"
            size="inline"
            onClick={() => onReset(shortcut.id)}
            className="opacity-0 group-hover:opacity-100 transition-opacity"
          >
            Reset
          </Button>
        )}
        <button
          onClick={() => onRebind(shortcut.id)}
          className="inline-flex items-center justify-center rounded border px-1.5 py-1 keybind-display text-xs min-w-[56px] transition-colors border-border bg-background text-foreground hover:bg-muted cursor-pointer"
        >
          {displayBinding}
        </button>
      </div>
    </div>
  );
}

function RecordingRow({
  shortcut,
  onCancel,
  onCapture,
}: {
  shortcut: ShortcutDefinition;
  onCancel: () => void;
  onCapture: (binding: string) => void;
}) {
  const rowRef = useRef<HTMLDivElement>(null);
  const onCancelRef = useRef(onCancel);
  const onCaptureRef = useRef(onCapture);
  onCancelRef.current = onCancel;
  onCaptureRef.current = onCapture;

  const pendingRef = useRef<string | null>(null);
  const [pendingDisplay, setPendingDisplay] = useState<string | null>(null);

  useEffect(() => {
    shortcutCaptureActive.current = true;
    suspendGlobalShortcuts();

    function onKeyDown(e: KeyboardEvent) {
      if (e.repeat) return;
      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape") {
        onCancelRef.current();
        return;
      }

      const binding = shortcut.isGlobal
        ? eventToGlobalBinding(e)
        : eventToBinding(e);
      if (binding) {
        pendingRef.current = binding;
        setPendingDisplay(
          shortcut.isGlobal
            ? formatGlobalShortcutDisplay(binding)
            : formatShortcutDisplay(binding),
        );
      }
    }

    function onKeyUp(e: KeyboardEvent) {
      e.preventDefault();
      e.stopPropagation();
      if (
        !e.metaKey &&
        !e.ctrlKey &&
        !e.shiftKey &&
        !e.altKey &&
        pendingRef.current
      ) {
        onCaptureRef.current(pendingRef.current);
      }
    }

    function onMouseDown(e: MouseEvent) {
      if (rowRef.current && !rowRef.current.contains(e.target as Node)) {
        onCancelRef.current();
      }
    }

    function onWindowBlur() {
      // If the user already pressed a combo, save it — blur may be
      // involuntary (e.g. a global shortcut opened another window).
      if (pendingRef.current) {
        onCaptureRef.current(pendingRef.current);
      } else {
        onCancelRef.current();
      }
    }

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("mousedown", onMouseDown, true);
    window.addEventListener("blur", onWindowBlur);
    return () => {
      shortcutCaptureActive.current = false;
      resumeGlobalShortcuts();
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("mousedown", onMouseDown, true);
      window.removeEventListener("blur", onWindowBlur);
    };
  }, [shortcut.isGlobal]);

  return (
    <div ref={rowRef} className="flex items-center justify-between py-1.5">
      <div className="flex flex-col gap-0.5 min-w-0">
        <span className="text-xs font-medium">{shortcut.label}</span>
        <span className="text-[11px] text-muted-foreground">
          {shortcut.description}
        </span>
      </div>
      <div className="flex items-center gap-1.5">
        <span className="inline-flex items-center justify-center rounded border-2 border-primary bg-primary/5 px-1.5 py-1 keybind-display text-xs min-w-[56px] animate-pulse text-primary">
          {pendingDisplay ?? "Press keys..."}
        </span>
      </div>
    </div>
  );
}

export function ShortcutsTab() {
  const overrides = useAppStore((s) => s.settings.shortcutBindings);
  const dictationSlots = useAppStore((s) => s.settings.dictation.slots);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const [recordingId, setRecordingId] = useState<string | null>(null);

  const handleRebind = useCallback((id: string) => {
    setRecordingId(id);
  }, []);

  const handleCancel = useCallback(() => {
    setRecordingId(null);
  }, []);

  const handleCapture = useCallback(
    (binding: string) => {
      if (!recordingId) return;

      // Determine if the recording shortcut is global
      const staticShortcut = SHORTCUTS.find((s) => s.id === recordingId);
      const isGlobal = staticShortcut?.isGlobal ?? recordingId.startsWith("global.");

      const conflict = findShortcutConflict(
        binding,
        recordingId,
        isGlobal,
        overrides,
        dictationSlots,
      );
      if (conflict) {
        const target =
          conflict.kind === "dictation"
            ? `dictation slot "${conflict.label}"`
            : `"${conflict.label}"`;
        toast.error(
          `${formatGlobalShortcutDisplay(binding)} is already bound to ${target}. Rebind it first.`,
        );
        setRecordingId(null);
        return;
      }

      updateSettings({ shortcutBindings: { ...overrides, [recordingId]: binding } });
      setRecordingId(null);
    },
    [recordingId, overrides, dictationSlots, updateSettings],
  );

  const handleReset = useCallback(
    (id: string) => {
      const next = { ...overrides };
      delete next[id];
      updateSettings({ shortcutBindings: next });
    },
    [overrides, updateSettings],
  );

  const handleResetAll = useCallback(() => {
    updateSettings({ shortcutBindings: {} });
  }, [updateSettings]);

  // Group shortcuts by category (excluding "Dictation" — handled separately)
  const groups = SHORTCUT_CATEGORIES
    .filter((cat) => cat !== "Dictation")
    .map((cat) => ({
      category: cat,
      shortcuts: SHORTCUTS.filter((s) => s.category === cat),
    }))
    .filter((g) => g.shortcuts.length > 0);

  // Build dynamic dictation shortcut definitions from store slots
  const dictationShortcuts: ShortcutDefinition[] = dictationSlots
    .filter((slot) => slot.enabled)
    .map((slot) => ({
      id: `global.dictation-${slot.id}`,
      label: slot.name,
      description: "Hold to dictate",
      category: "Dictation" as const,
      defaultBinding: slot.defaultBinding ?? "",
      isGlobal: true,
      isDictation: true,
    }));

  const hasAnyOverride = Object.keys(overrides).length > 0;

  return (
    <div className="space-y-6 p-4">
      {/* Header with reset all */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-xs font-medium">Keyboard Shortcuts</h3>
          <p className="text-xs text-muted-foreground mt-0.5">
            Click a binding to rebind. Global shortcuts work even when the app is in the background.
          </p>
        </div>
        {hasAnyOverride && (
          <Button
            variant="ghost"
            size="sm"
            onClick={handleResetAll}
            className="text-xs text-muted-foreground"
          >
            <RotateCcw className="h-3 w-3 mr-1" />
            Reset All
          </Button>
        )}
      </div>

      {groups.map((group) => (
        <div key={group.category}>
          <h4 className="text-[11px] font-medium uppercase text-muted-foreground mb-2">
            {group.category}
          </h4>
          <div className="divide-y divide-border">
            {group.shortcuts.map((shortcut) => {
              if (recordingId === shortcut.id) {
                return (
                  <RecordingRow
                    key={shortcut.id}
                    shortcut={shortcut}
                    onCancel={handleCancel}
                    onCapture={handleCapture}
                  />
                );
              }

              const binding = getBinding(shortcut.id, overrides);

              return (
                <ShortcutRow
                  key={shortcut.id}
                  shortcut={shortcut}
                  binding={binding}
                  onRebind={handleRebind}
                  onReset={handleReset}
                  hasOverride={shortcut.id in overrides}
                  conflict={null}
                />
              );
            })}
          </div>
        </div>
      ))}

      {/* Dictation section from dynamic store slots */}
      {dictationShortcuts.length > 0 && (
        <div>
          <h4 className="text-[11px] font-medium uppercase text-muted-foreground mb-2">
            Dictation
          </h4>
          <div className="divide-y divide-border">
            {dictationShortcuts.map((shortcut) => {
              if (recordingId === shortcut.id) {
                return (
                  <RecordingRow
                    key={shortcut.id}
                    shortcut={shortcut}
                    onCancel={handleCancel}
                    onCapture={handleCapture}
                  />
                );
              }

              const binding = overrides[shortcut.id] ?? shortcut.defaultBinding ?? "";

              return (
                <ShortcutRow
                  key={shortcut.id}
                  shortcut={shortcut}
                  binding={binding}
                  onRebind={handleRebind}
                  onReset={handleReset}
                  hasOverride={shortcut.id in overrides}
                  conflict={null}
                />
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
