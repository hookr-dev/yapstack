import { useState, useCallback, useEffect, useRef } from "react";
import { useDebouncedCallback } from "use-debounce";
import { useAppStore } from "@/stores/appStore";
import type { DictationSlot, DictationOutputAction, DictationActivationMode } from "@/stores/appStore";
import { trackDictationSlotCreated, trackDictationSlotDeleted, trackDictationSlotConfigured } from "@/lib/analytics";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Slider } from "@/components/ui/slider";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  eventToGlobalBinding,
  findShortcutConflict,
  shortcutCaptureActive,
} from "@/lib/shortcuts";
import { toast } from "sonner";
import { suspendGlobalShortcuts, resumeGlobalShortcuts } from "@/hooks/useGlobalShortcuts";
import { formatGlobalShortcutDisplay } from "@/lib/utils";
import { Plus, Trash2 } from "lucide-react";

const OUTPUT_ACTION_LABELS: Record<DictationOutputAction, string> = {
  paste: "Paste",
  clipboard: "Clipboard Only",
  "new-note": "New Note",
};

function KeybindRecorder({
  slotId,
  binding,
  onCapture,
}: {
  slotId: string;
  binding: string;
  onCapture: (slotId: string, binding: string) => void;
}) {
  const [recording, setRecording] = useState(false);

  // We need useEffect for the keydown listener
  // but since we're using useState for recording, we handle it inline
  if (recording) {
    return (
      <KeybindCaptureInline
        onCapture={(b) => {
          onCapture(slotId, b);
          setRecording(false);
        }}
        onCancel={() => setRecording(false)}
      />
    );
  }

  return (
    <button
      onClick={() => setRecording(true)}
      className="inline-flex items-center justify-center rounded border px-1.5 py-1 keybind-display text-xs min-w-[56px] transition-colors border-border bg-background text-foreground hover:bg-muted cursor-pointer"
    >
      {formatGlobalShortcutDisplay(binding)}
    </button>
  );
}

function KeybindCaptureInline({
  onCapture,
  onCancel,
}: {
  onCapture: (binding: string) => void;
  onCancel: () => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const onCaptureRef = useRef(onCapture);
  const onCancelRef = useRef(onCancel);
  onCaptureRef.current = onCapture;
  onCancelRef.current = onCancel;

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

      const captured = eventToGlobalBinding(e);
      if (captured) {
        pendingRef.current = captured;
        setPendingDisplay(formatGlobalShortcutDisplay(captured));
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
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
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
  }, []);

  return (
    <div ref={containerRef} className="flex items-center gap-1">
      <span className="inline-flex items-center justify-center rounded border-2 border-primary bg-primary/5 px-1.5 py-1 keybind-display text-xs min-w-[56px] animate-pulse text-primary">
        {pendingDisplay ?? "Press keys..."}
      </span>
    </div>
  );
}

function SlotCard({
  slot,
  binding,
  onUpdate,
  onKeybindCapture,
  onDelete,
}: {
  slot: DictationSlot;
  binding: string;
  onUpdate: (id: string, updates: Partial<DictationSlot>) => void;
  onKeybindCapture: (slotId: string, binding: string) => void;
  onDelete: (id: string) => void;
}) {
  return (
    <div className="rounded-lg border p-3 space-y-2">
      {/* Row 1: name input + delete */}
      <div className="flex items-center justify-between gap-2">
        <input
          type="text"
          value={slot.name}
          onChange={(e) => onUpdate(slot.id, { name: e.target.value })}
          className="h-6 text-xs font-medium rounded border border-border bg-transparent px-2 outline-none max-w-[160px] focus:border-primary transition-colors"
        />
        <button
          onClick={() => onDelete(slot.id)}
          className="p-0.5 text-muted-foreground/50 hover:text-destructive transition-colors"
          title="Delete slot"
        >
          <Trash2 className="h-3 w-3" />
        </button>
      </div>

      {/* Row 2: keybind + output + AI toggle on one line */}
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-[11px] text-muted-foreground shrink-0">
            Keybind
          </span>
          <KeybindRecorder
            slotId={`global.dictation-${slot.id}`}
            binding={binding}
            onCapture={onKeybindCapture}
          />
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-[11px] text-muted-foreground">
            Output
          </span>
          <Select
            value={slot.outputAction ?? "paste"}
            onValueChange={(v) =>
              onUpdate(slot.id, { outputAction: v as DictationOutputAction })
            }
          >
            <SelectTrigger className="!h-6 w-[110px] text-[11px] px-2 py-0">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="paste" className="text-[11px]">
                {OUTPUT_ACTION_LABELS.paste}
              </SelectItem>
              <SelectItem value="clipboard" className="text-[11px]">
                {OUTPUT_ACTION_LABELS.clipboard}
              </SelectItem>
              <SelectItem value="new-note" className="text-[11px]">
                {OUTPUT_ACTION_LABELS["new-note"]}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-[11px] text-muted-foreground">AI</span>
          <Switch
            size="sm"
            checked={slot.aiEnabled}
            onCheckedChange={(checked) =>
              onUpdate(slot.id, { aiEnabled: checked })
            }
          />
        </div>
      </div>

      {/* Row 3 (conditional): AI prompt */}
      {slot.aiEnabled && (
        <textarea
          value={slot.prompt}
          onChange={(e) => onUpdate(slot.id, { prompt: e.target.value })}
          placeholder="System prompt for AI processing..."
          rows={3}
          className="w-full rounded-md border bg-muted/50 px-2.5 py-1.5 text-xs outline-none resize-none focus:border-primary transition-colors placeholder:text-muted-foreground/50"
        />
      )}
    </div>
  );
}

export function DictationTab() {
  const dictation = useAppStore((s) => s.settings.dictation);
  const shortcutBindings = useAppStore((s) => s.settings.shortcutBindings);
  const dictationDuckEnabled = useAppStore((s) => s.settings.dictationDuckEnabled);
  const dictationDuckAmount = useAppStore((s) => s.settings.dictationDuckAmount);
  const updateSettings = useAppStore((s) => s.updateSettings);

  const handleToggleEnabled = (checked: boolean) => {
    updateSettings({
      dictation: { ...dictation, enabled: checked },
    });
  };

  // Coalesce per-keystroke edits to slot fields into one analytics
  // event per ~2 s edit session. The accumulator lives in a ref so
  // typing doesn't re-render; the debounced callback fires the event
  // with the union of changed field names. `flush()` is invoked on
  // unmount and on pagehide so an event can't be lost when the user
  // closes the window before the timer elapses (Aptabase's Tauri-side
  // exit flush only flushes events already enqueued via `track()`).
  const slotConfigFieldsRef = useRef<Set<string>>(new Set());
  const flushSlotConfigured = useDebouncedCallback(() => {
    if (slotConfigFieldsRef.current.size === 0) return;
    trackDictationSlotConfigured({
      changed_fields: Array.from(slotConfigFieldsRef.current).join(","),
    });
    slotConfigFieldsRef.current.clear();
  }, 2000);

  useEffect(() => {
    const onPageHide = () => flushSlotConfigured.flush();
    window.addEventListener("pagehide", onPageHide);
    return () => {
      window.removeEventListener("pagehide", onPageHide);
      flushSlotConfigured.flush();
    };
  }, [flushSlotConfigured]);

  const handleSlotUpdate = (id: string, updates: Partial<DictationSlot>) => {
    const newSlots = dictation.slots.map((s) =>
      s.id === id ? { ...s, ...updates } : s,
    );
    updateSettings({
      dictation: { ...dictation, slots: newSlots },
    });
    for (const k of Object.keys(updates)) slotConfigFieldsRef.current.add(k);
    flushSlotConfigured();
  };

  const handleKeybindCapture = useCallback(
    (shortcutId: string, binding: string) => {
      const conflict = findShortcutConflict(
        binding,
        shortcutId,
        true, // dictation slot bindings are always global
        shortcutBindings,
        dictation.slots,
      );
      if (conflict) {
        const target =
          conflict.kind === "dictation"
            ? `dictation slot "${conflict.label}"`
            : `"${conflict.label}"`;
        toast.error(
          `${formatGlobalShortcutDisplay(binding)} is already bound to ${target}. Rebind it first.`,
        );
        return;
      }
      const newOverrides = { ...shortcutBindings, [shortcutId]: binding };
      updateSettings({ shortcutBindings: newOverrides });
    },
    [shortcutBindings, dictation.slots, updateSettings],
  );

  const handleAddSlot = () => {
    const newSlot: DictationSlot = {
      id: crypto.randomUUID(),
      name: `Slot ${dictation.slots.length + 1}`,
      enabled: true,
      aiEnabled: false,
      prompt: "",
      outputAction: "paste",
    };
    updateSettings({
      dictation: { ...dictation, slots: [...dictation.slots, newSlot] },
    });
    trackDictationSlotCreated();
  };

  const handleDeleteSlot = (id: string) => {
    const keybindKey = `global.dictation-${id}`;
    const hasKeybind = !!shortcutBindings[keybindKey];

    if (hasKeybind && !window.confirm("This slot has a keybind assigned. Delete it?")) {
      return;
    }

    const newSlots = dictation.slots.filter((s) => s.id !== id);
    const newOverrides = { ...shortcutBindings };
    delete newOverrides[keybindKey];

    updateSettings({
      dictation: { ...dictation, slots: newSlots },
      shortcutBindings: newOverrides,
    });
    trackDictationSlotDeleted();
  };

  return (
    <>
      {/* Main toggle */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-xs font-medium">Voice Dictation</h3>
          <p className="text-xs text-muted-foreground mt-0.5">
            Hold a keybind to dictate, release to transcribe. Captures
            your microphone only — system audio is never included.
          </p>
        </div>
        <Switch
          checked={dictation.enabled}
          onCheckedChange={handleToggleEnabled}
        />
      </div>

      {dictation.enabled && (
        <>
          <Separator />

          {/* Activation mode */}
          <div className="flex items-center justify-between">
            <div>
              <Label className="text-xs">Activation Mode</Label>
              <p className="text-[11px] text-muted-foreground mt-0.5">
                {dictation.activationMode === "toggle"
                  ? "Press keybind once to start, press again to stop."
                  : "Hold keybind to record, release to stop."}
              </p>
            </div>
            <Select
              value={dictation.activationMode ?? "hold"}
              onValueChange={(v) =>
                updateSettings({
                  dictation: { ...dictation, activationMode: v as DictationActivationMode },
                })
              }
            >
              <SelectTrigger className="w-[100px] text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="hold" className="text-xs">Hold</SelectItem>
                <SelectItem value="toggle" className="text-xs">Toggle</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <Separator />

          {/* Volume Duck */}
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <Label className="text-xs">Lower volume while recording</Label>
                <p className="text-[11px] text-muted-foreground mt-0.5">
                  Quiets system audio so you can hear yourself.
                </p>
              </div>
              <Switch
                checked={dictationDuckEnabled}
                onCheckedChange={(checked) =>
                  updateSettings({ dictationDuckEnabled: checked })
                }
              />
            </div>
            <div className="space-y-1.5">
              <div className="flex items-center justify-between">
                <Label className="text-[11px] text-muted-foreground">Reduce volume by</Label>
                <span className="text-[11px] text-muted-foreground tabular-nums">
                  {Math.round(dictationDuckAmount * 100)}%
                </span>
              </div>
              <Slider
                min={0}
                max={100}
                step={5}
                value={[Math.round(dictationDuckAmount * 100)]}
                disabled={!dictationDuckEnabled}
                onValueChange={(values) => {
                  const pct = values[0] ?? 80;
                  updateSettings({ dictationDuckAmount: pct / 100 });
                }}
              />
            </div>
          </div>

          <Separator />

          {/* Slots */}
          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">
              Dictation Slots
            </Label>
            <div className="space-y-2">
              {dictation.slots.map((slot) => (
                <SlotCard
                  key={slot.id}
                  slot={slot}
                  binding={shortcutBindings[`global.dictation-${slot.id}`] ?? ""}
                  onUpdate={handleSlotUpdate}
                  onKeybindCapture={handleKeybindCapture}
                  onDelete={handleDeleteSlot}
                />
              ))}
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={handleAddSlot}
              className="w-full text-xs"
            >
              <Plus className="h-3 w-3 mr-1" />
              Add Slot
            </Button>
          </div>
        </>
      )}
    </>
  );
}
