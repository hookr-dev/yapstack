import { useAppStore } from "@/stores/appStore";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ProfilePicker } from "@/components/ai/ProfilePicker";
import { Trash2 } from "lucide-react";
import type { Insight } from "@/lib/insights";
import { HEARTBEAT_PRESETS } from "@/lib/insights";

export function InsightCard({
  insight,
  onUpdate,
  onDelete,
}: {
  insight: Insight;
  onUpdate: (id: string, updates: Partial<Insight>) => void;
  onDelete: (id: string) => void;
}) {
  const aiConfig = useAppStore((s) => s.settings.aiConfig);

  return (
    <div className="rounded-lg border p-3 space-y-2">
      {/* Row 1: name input + delete */}
      <div className="flex items-center justify-between gap-2">
        <input
          type="text"
          value={insight.name}
          onChange={(e) => onUpdate(insight.id, { name: e.target.value })}
          className="h-6 text-xs font-medium rounded border border-border bg-transparent px-2 outline-none max-w-[200px] focus:border-primary transition-colors"
        />
        <button
          onClick={() => onDelete(insight.id)}
          className="p-0.5 text-muted-foreground/50 hover:text-destructive transition-colors"
          title="Delete insight"
        >
          <Trash2 className="h-3 w-3" />
        </button>
      </div>

      {/* Row 2: enabled + heartbeat + AI profile pill */}
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-[11px] text-muted-foreground">Enabled</span>
          <Switch
            size="sm"
            checked={insight.enabled}
            onCheckedChange={(checked) =>
              onUpdate(insight.id, { enabled: checked })
            }
          />
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-[11px] text-muted-foreground">Every</span>
          <Select
            value={String(insight.heartbeatSeconds)}
            onValueChange={(v) =>
              onUpdate(insight.id, { heartbeatSeconds: Number(v) })
            }
          >
            <SelectTrigger className="!h-6 w-[80px] text-[11px] px-2 py-0">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {HEARTBEAT_PRESETS.map((p) => (
                <SelectItem
                  key={p.value}
                  value={String(p.value)}
                  className="text-[11px]"
                >
                  {p.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-[11px] text-muted-foreground">AI</span>
          <ProfilePicker
            profiles={aiConfig.profiles}
            connections={aiConfig.connections}
            value={insight.profileId}
            onChange={(next) => onUpdate(insight.id, { profileId: next })}
            allowNone
            noneLabel="None — no model assigned"
            unassignedLabel="None"
            variant="pill"
          />
        </div>
      </div>

      {/* Row 3: prompt textarea (always shown — the prompt is the feature) */}
      <textarea
        value={insight.prompt}
        onChange={(e) => onUpdate(insight.id, { prompt: e.target.value })}
        placeholder="System prompt — describe what this insight should extract from the live transcript (e.g. 'list any acronyms or jargon a non-specialist might miss')…"
        rows={3}
        className="w-full rounded-md border bg-muted/50 px-2.5 py-1.5 text-xs outline-none resize-none focus:border-primary transition-colors placeholder:text-muted-foreground/50"
      />
    </div>
  );
}
