import { useState } from "react";
import { ChevronDown, Sparkles, Check } from "lucide-react";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";

/**
 * Compact dropdown rendered in the Insight overlay's header strip. Lets the
 * user switch the Current Insight without opening Settings. Emits the new
 * id via `onChange`; the controller hook in the main window receives the
 * corresponding Tauri event and writes the change to the runtime store
 * (NOT the persisted Default — switching here is per-session only).
 *
 * `data-tauri-drag-region="false"` on the trigger + popover keeps the
 * surrounding `<header data-tauri-drag-region>` draggable everywhere except
 * the picker itself — clicks land on the picker, drags happen elsewhere.
 *
 * There is no "None — turn overlay off" entry: that's the × button's job.
 * The dropdown's purpose is purely "switch to a different Insight."
 */
export function InsightSwitcher({
  currentName,
  slots,
  currentInsightId,
  onChange,
  onOpenChange,
}: {
  currentName: string;
  slots: { id: string; name: string }[];
  currentInsightId: string | null;
  onChange: (next: string) => void;
  /** Notified whenever the popover opens or closes. The overlay uses this
   *  to suspend its cursor-position-driven click-through toggle while the
   *  dropdown is open — otherwise clicks on popover items would fall
   *  through to the window beneath. */
  onOpenChange?: (open: boolean) => void;
}) {
  const [open, setOpen] = useState(false);

  function handleOpenChange(next: boolean) {
    setOpen(next);
    onOpenChange?.(next);
  }

  function pick(id: string) {
    onChange(id);
    handleOpenChange(false);
  }

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger asChild>
        <button
          type="button"
          data-tauri-drag-region="false"
          className="inline-flex min-w-0 items-center gap-1 rounded px-1 py-0.5 text-[11px] font-medium text-foreground/90 hover:bg-muted/40 hover:text-foreground transition-colors"
        >
          <Sparkles className="h-3 w-3 shrink-0 text-muted-foreground" />
          <span className="truncate">{currentName || "Insight"}</span>
          <ChevronDown className="h-2.5 w-2.5 shrink-0 text-muted-foreground/70" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="bottom"
        align="start"
        sideOffset={6}
        collisionPadding={8}
        className="w-56 max-h-[60vh] overflow-y-auto p-1"
        data-tauri-drag-region="false"
      >
        {slots.length === 0 ? (
          <p className="px-3 py-3 text-center text-[10px] text-muted-foreground">
            No enabled insights. Configure in Settings → AI → Insights.
          </p>
        ) : (
          slots.map((s) => (
            <PickerItem
              key={s.id}
              label={s.name}
              selected={currentInsightId === s.id}
              onClick={() => pick(s.id)}
            />
          ))
        )}
      </PopoverContent>
    </Popover>
  );
}

function PickerItem({
  label,
  selected,
  onClick,
}: {
  label: string;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex w-full items-center justify-between gap-2 rounded px-2 py-1.5 text-xs text-left transition-colors",
        "hover:bg-accent hover:text-accent-foreground",
        !selected && "text-foreground",
        selected && "font-medium text-foreground",
      )}
    >
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {selected && <Check className="h-3 w-3 shrink-0 opacity-70" />}
    </button>
  );
}
