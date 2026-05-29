import { useAppStore } from "@/stores/appStore";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ProfilePicker } from "@/components/ai/ProfilePicker";
import { Trash2 } from "lucide-react";
import {
  CADENCE_PRESET_OPTIONS,
  INSIGHT_TYPE_OPTIONS,
  applyTemplate,
  describeTrigger,
  resolveTriggerConfig,
  type CadencePreset,
  type Insight,
  type InsightType,
  type TriggerConfig,
} from "@/lib/insights";

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
  const isCustom = insight.trigger.preset === "custom";

  const handleTypeChange = (next: InsightType) => {
    if (next === "custom") {
      // Selecting Custom just marks it freeform — leave the prompt + cadence as-is.
      onUpdate(insight.id, { type: "custom" });
    } else {
      // Seed the starter prompt + recommended cadence for this type.
      onUpdate(insight.id, applyTemplate(next));
    }
  };

  const handlePresetChange = (next: CadencePreset) => {
    if (next === "custom") {
      // Seed the custom knobs from whatever was effective before, so the user
      // starts editing from their current cadence rather than a blank slate.
      onUpdate(insight.id, {
        trigger: { preset: "custom", ...resolveTriggerConfig(insight) },
      });
    } else {
      // Keep the stored numbers (harmless — the engine reads the preset's
      // values); just flip the preset.
      onUpdate(insight.id, { trigger: { ...insight.trigger, preset: next } });
    }
  };

  const setKnob = (field: keyof TriggerConfig, value: number) => {
    onUpdate(insight.id, {
      trigger: { ...insight.trigger, preset: "custom", [field]: value },
    });
  };

  return (
    <div className="rounded-lg border p-3 space-y-2">
      {/* Row 1: name input + delete */}
      <div className="flex items-center justify-between gap-2">
        <input
          type="text"
          value={insight.name}
          onChange={(e) => onUpdate(insight.id, { name: e.target.value })}
          className="h-6 text-xs font-medium rounded border border-border bg-transparent px-2 outline-none max-w-[160px] focus:border-primary transition-colors"
        />
        <button
          onClick={() => onDelete(insight.id)}
          className="p-0.5 text-muted-foreground/50 hover:text-destructive transition-colors"
          title="Delete insight"
        >
          <Trash2 className="h-3 w-3" />
        </button>
      </div>

      {/* Row 2: type + cadence + AI profile — spread edge-to-edge like the
          Dictation slot card's controls row (see docs/FRONTEND.md). */}
      <div className="space-y-1.5">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-1.5 shrink-0">
            <span className="text-[11px] text-muted-foreground">Type</span>
            <Select
              value={insight.type}
              onValueChange={(v) => handleTypeChange(v as InsightType)}
            >
              <SelectTrigger className="!h-6 w-[130px] text-[11px] px-2 py-0">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {INSIGHT_TYPE_OPTIONS.map((o) => (
                  <SelectItem
                    key={o.value}
                    value={o.value}
                    className="text-[11px]"
                  >
                    {o.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-center gap-1.5 shrink-0">
            <span className="text-[11px] text-muted-foreground">Cadence</span>
            <Select
              value={insight.trigger.preset}
              onValueChange={(v) => handlePresetChange(v as CadencePreset)}
            >
              <SelectTrigger className="!h-6 w-[110px] text-[11px] px-2 py-0">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CADENCE_PRESET_OPTIONS.map((o) => (
                  <SelectItem
                    key={o.value}
                    value={o.value}
                    className="text-[11px]"
                  >
                    {o.label}
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

        {isCustom && (
          <div className="grid grid-cols-2 gap-x-3 gap-y-1 rounded-md border border-dashed border-border/70 px-2.5 py-2">
            <Knob
              label="Threshold"
              suffix="words"
              step={10}
              value={insight.trigger.thresholdWords}
              onChange={(v) => setKnob("thresholdWords", v)}
            />
            <Knob
              label="Settle"
              suffix="s"
              step={0.5}
              value={insight.trigger.settleSeconds}
              onChange={(v) => setKnob("settleSeconds", v)}
            />
            <Knob
              label="Min gap"
              suffix="s"
              step={1}
              value={insight.trigger.minIntervalSeconds}
              onChange={(v) => setKnob("minIntervalSeconds", v)}
            />
            <Knob
              label="Max wait"
              suffix="s"
              step={5}
              value={insight.trigger.maxWaitSeconds}
              onChange={(v) => setKnob("maxWaitSeconds", v)}
            />
          </div>
        )}

        <p className="text-[11px] text-muted-foreground/80">
          {describeTrigger(resolveTriggerConfig(insight))}
        </p>
      </div>

      {/* Row 4: prompt textarea (always shown — the prompt is the feature).
          Editing a templated prompt flips the type to Custom, mirroring how
          editing a cadence knob flips the cadence preset to Custom. */}
      <textarea
        value={insight.prompt}
        onChange={(e) =>
          onUpdate(insight.id, { prompt: e.target.value, type: "custom" })
        }
        placeholder="System prompt — describe what this insight should extract from the live transcript (e.g. 'list any acronyms or jargon a non-specialist might miss')…"
        rows={3}
        className="w-full rounded-md border bg-muted/50 px-2.5 py-1.5 text-xs outline-none resize-none focus:border-primary transition-colors placeholder:text-muted-foreground/50"
      />
    </div>
  );
}

/** Compact labeled number input for the Custom cadence knobs. */
function Knob({
  label,
  suffix,
  step,
  value,
  onChange,
}: {
  label: string;
  suffix: string;
  step: number;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-1.5">
      <span className="text-[11px] text-muted-foreground">{label}</span>
      <span className="flex items-center gap-1">
        <Input
          type="number"
          min={0}
          step={step}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
          className="h-6 w-14 px-1.5 text-[11px] text-right"
        />
        <span className="w-8 text-[10px] text-muted-foreground/70">
          {suffix}
        </span>
      </span>
    </label>
  );
}
