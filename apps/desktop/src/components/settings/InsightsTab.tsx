import { useAppStore } from "@/stores/appStore";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Plus, Sparkles } from "lucide-react";
import { InsightCard } from "@/components/insights/InsightCard";
import { buildInsight, type Insight } from "@/lib/insights";

const DEFAULT_INSIGHT_NONE = "__none__";

export function InsightsTab() {
  const insights = useAppStore((s) => s.settings.insights);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const setCurrentInsightId = useAppStore((s) => s.setCurrentInsightId);

  const handleToggleEnabled = (checked: boolean) => {
    updateSettings({ insights: { ...insights, enabled: checked } });
  };

  const handleDefaultChange = (value: string) => {
    updateSettings({
      insights: {
        ...insights,
        defaultInsightId: value === DEFAULT_INSIGHT_NONE ? null : value,
      },
    });
  };

  const handleSlotUpdate = (id: string, updates: Partial<Insight>) => {
    const newSlots = insights.slots.map((s) =>
      s.id === id ? { ...s, ...updates } : s,
    );
    updateSettings({ insights: { ...insights, slots: newSlots } });
  };

  const handleAddSlot = () => {
    const next = buildInsight(`Insight ${insights.slots.length + 1}`);
    updateSettings({
      insights: { ...insights, slots: [...insights.slots, next] },
    });
  };

  const handleDeleteSlot = (id: string) => {
    const newSlots = insights.slots.filter((s) => s.id !== id);
    // If the deleted Insight was the Default, clear the Default so future
    // sessions don't try to auto-load a non-existent slot.
    const newDefaultId =
      insights.defaultInsightId === id ? null : insights.defaultInsightId;
    updateSettings({
      insights: {
        ...insights,
        slots: newSlots,
        defaultInsightId: newDefaultId,
      },
    });
    // If the deleted Insight is the one running right now, clear the runtime
    // Current Insight too. The overlay gate already hides on a missing slot,
    // but this avoids leaving a dangling id in session state.
    if (useAppStore.getState().currentInsightId === id) {
      setCurrentInsightId(null);
    }
  };

  const hasSlots = insights.slots.length > 0;
  const enabledSlots = insights.slots.filter((s) => s.enabled);

  return (
    <>
      {/* Master toggle */}
      <div className="flex items-center justify-between gap-6">
        <div className="min-w-0 max-w-prose">
          <h3 className="text-xs font-medium">Live insights</h3>
          <p className="text-xs text-muted-foreground mt-0.5">
            Surface AI commentary on the live transcript while a session is
            running.
          </p>
        </div>
        <Switch
          checked={insights.enabled}
          onCheckedChange={handleToggleEnabled}
        />
      </div>

      {insights.enabled && (
        <>
          <Separator />

          {/* Default Insight picker — drives session-start auto-load only. */}
          <div className="flex items-center justify-between gap-6">
            <div className="min-w-0 max-w-prose">
              <Label className="text-xs">Default Insight</Label>
              <p className="text-[11px] text-muted-foreground mt-0.5">
                Auto-loads at the start of each session. Overlay switches are
                session-only and don&rsquo;t change this.
              </p>
            </div>
            <Select
              value={insights.defaultInsightId ?? DEFAULT_INSIGHT_NONE}
              onValueChange={handleDefaultChange}
            >
              <SelectTrigger className="w-[200px] text-xs">
                <SelectValue placeholder="None" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={DEFAULT_INSIGHT_NONE} className="text-xs">
                  None — no auto-start
                </SelectItem>
                {enabledSlots.map((s) => (
                  <SelectItem key={s.id} value={s.id} className="text-xs">
                    {s.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <Separator />

          {/* Slots list */}
          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">Insights</Label>
            {hasSlots ? (
              <div className="space-y-2">
                {insights.slots.map((insight) => (
                  <InsightCard
                    key={insight.id}
                    insight={insight}
                    onUpdate={handleSlotUpdate}
                    onDelete={handleDeleteSlot}
                  />
                ))}
              </div>
            ) : (
              <EmptyState onAdd={handleAddSlot} />
            )}
            {hasSlots && (
              <Button
                variant="outline"
                size="sm"
                onClick={handleAddSlot}
                className="w-full text-xs"
              >
                <Plus className="h-3 w-3 mr-1" />
                Add Insight
              </Button>
            )}
          </div>
        </>
      )}
    </>
  );
}

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="rounded-md border border-dashed border-border px-6 py-5 text-center">
      <div className="mx-auto mb-2 flex h-7 w-7 items-center justify-center rounded-full bg-muted">
        <Sparkles className="h-3 w-3 text-muted-foreground" />
      </div>
      <p className="mx-auto max-w-xs text-[11px] leading-relaxed text-muted-foreground">
        No Insights yet. Each Insight bundles a system prompt and a heartbeat
        — assign one to feed the overlay during a live session.
      </p>
      <Button size="sm" onClick={onAdd} className="mt-3 text-xs">
        <Plus className="mr-1 h-3 w-3" />
        Add your first Insight
      </Button>
    </div>
  );
}
