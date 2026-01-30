import { useState } from "react";
import { useAppStore } from "@/stores/appStore";
import {
  Popover,
  PopoverTrigger,
  PopoverContent,
} from "@/components/ui/popover";
import { Check, ChevronDown, Zap } from "lucide-react";
import { getAllModelsGrouped, DEFAULT_AI_SETTINGS } from "@/lib/ai";
import { cn } from "@/lib/utils";

export function ModelPickerPill() {
  const ai = useAppStore((s) => s.settings.ai) ?? DEFAULT_AI_SETTINGS;
  const updateSettings = useAppStore((s) => s.updateSettings);
  const provider = ai.activeProvider;
  const config = ai.providers[provider];
  const groups = getAllModelsGrouped(provider);
  const [open, setOpen] = useState(false);

  const currentLabel = (() => {
    for (const g of groups) {
      const found = g.models.find((m) => m.id === config.model);
      if (found) return found.label;
    }
    return config.model;
  })();

  function selectModel(modelId: string) {
    updateSettings({
      ai: {
        ...ai,
        providers: {
          ...ai.providers,
          [provider]: { ...config, model: modelId },
        },
      },
    });
    setOpen(false);
  }

  if (groups.length === 0) {
    return (
      <span className="inline-flex items-center gap-1 rounded-md border border-muted-foreground/20 px-2 py-0.5 text-[9px] text-muted-foreground">
        <Zap className="h-2.5 w-2.5" />
        {currentLabel || "No model"}
      </span>
    );
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button className="inline-flex items-center gap-1 rounded-md border border-muted-foreground/20 px-2 py-0.5 text-[9px] text-muted-foreground hover:border-foreground/40 hover:text-foreground transition-colors">
          <Zap className="h-2.5 w-2.5" />
          {currentLabel}
          <ChevronDown className="h-2 w-2" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="top"
        align="start"
        className="w-56 max-h-[60vh] overflow-y-auto p-1"
        sideOffset={4}
        collisionPadding={8}
      >
        {groups.map((group, gi) => (
          <div key={group.provider}>
            {gi > 0 && <div className="border-t my-1" />}
            <div className="text-[9px] text-muted-foreground/50 uppercase px-2 pt-2 pb-1 select-none">
              {group.providerLabel}
            </div>
            {group.models.map((m) =>
              m.available ? (
                <button
                  key={m.id}
                  onClick={() => selectModel(m.id)}
                  className={cn(
                    "flex items-center justify-between w-full px-2 py-1.5 rounded text-xs hover:bg-accent transition-colors",
                    config.model === m.id
                      ? "text-foreground font-medium"
                      : "text-muted-foreground",
                  )}
                >
                  <span>{m.label}</span>
                  <span className="flex items-center gap-1">
                    {m.recommended && (
                      <span className="rounded-full bg-primary/10 px-1.5 py-0.5 text-[9px] text-primary">
                        Recommended
                      </span>
                    )}
                    {config.model === m.id && (
                      <Check className="h-3 w-3 text-primary" />
                    )}
                  </span>
                </button>
              ) : (
                <div
                  key={m.id}
                  className="flex items-center w-full px-2 py-1.5 text-xs text-muted-foreground opacity-40 cursor-default pointer-events-none"
                >
                  {m.label}
                </div>
              ),
            )}
          </div>
        ))}
      </PopoverContent>
    </Popover>
  );
}
