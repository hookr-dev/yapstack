import { useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ChevronsUpDown } from "lucide-react";
import { ModelSection } from "./ModelSection";

const CHUNK_DURATION_OPTIONS = [10, 15, 20, 30].map((d) => ({
  value: d,
  label: `${d}s`,
}));

const SILENCE_PAUSE_OPTIONS = [300, 500, 800, 1200].map((d) => ({
  value: d,
  label: `${(d / 1000).toFixed(1)}s`,
}));

const PROMPT_CONTEXT_OPTIONS = [0, 200, 350, 500].map((d) => ({
  value: d,
  label: d === 0 ? "Off" : `${d}`,
}));

const PROMPT_DECAY_OPTIONS = [
  { value: 0, label: "Off" },
  { value: 3, label: "3s" },
  { value: 5, label: "5s" },
  { value: 10, label: "10s" },
];

const LANGUAGES = [
  { value: "en", label: "English" },
  { value: "es", label: "Spanish" },
  { value: "fr", label: "French" },
  { value: "de", label: "German" },
  { value: "ja", label: "Japanese" },
  { value: "zh", label: "Chinese" },
  { value: "ko", label: "Korean" },
  { value: "pt", label: "Portuguese" },
  { value: "it", label: "Italian" },
  { value: "ru", label: "Russian" },
];

interface ButtonGroupOption {
  value: number;
  label: string;
}

function ButtonGroupSetting({
  label,
  description,
  options,
  currentValue,
  onChange,
}: {
  label: string;
  description: string;
  options: ButtonGroupOption[];
  currentValue: number;
  onChange: (value: number) => void;
}) {
  return (
    <div className="space-y-1.5">
      <span className="text-[11px] text-muted-foreground">{label}</span>
      <div className="flex gap-1.5">
        {options.map((opt) => (
          <Button
            key={opt.value}
            size="sm"
            variant={currentValue === opt.value ? "default" : "outline"}
            className="flex-1 text-xs"
            onClick={() => onChange(opt.value)}
          >
            {opt.label}
          </Button>
        ))}
      </div>
      <p className="text-[10px] text-muted-foreground/60">{description}</p>
    </div>
  );
}

export function TranscriptionTab() {
  const language = useAppStore((s) => s.settings.language);
  const maxChunkSeconds = useAppStore((s) => s.settings.maxChunkSeconds);
  const silenceDurationMs = useAppStore((s) => s.settings.silenceDurationMs);
  const promptContextChars = useAppStore((s) => s.settings.promptContextChars);
  const promptDecaySilenceSeconds = useAppStore((s) => s.settings.promptDecaySilenceSeconds);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const [showAdvanced, setShowAdvanced] = useState(false);

  return (
    <>
      {/* Language */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Language</Label>
        <Select
          value={language}
          onValueChange={(v) => updateSettings({ language: v })}
        >
          <SelectTrigger className="h-8 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {LANGUAGES.map((lang) => (
              <SelectItem
                key={lang.value}
                value={lang.value}
                className="text-xs"
              >
                {lang.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <Separator />

      {/* Model */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Model</Label>
        <ModelSection />
      </div>

      <Separator />

      {/* Advanced */}
      <Collapsible open={showAdvanced} onOpenChange={setShowAdvanced}>
        <CollapsibleTrigger className="flex w-full items-center justify-between">
          <Label className="text-xs text-muted-foreground pointer-events-none">
            Advanced
          </Label>
          <ChevronsUpDown className="h-3.5 w-3.5 text-muted-foreground/60" />
        </CollapsibleTrigger>
        <CollapsibleContent className="space-y-3 pt-3">
          <ButtonGroupSetting
            label="Max Chunk Duration"
            description="Longest chunk sent for transcription during continuous speech"
            options={CHUNK_DURATION_OPTIONS}
            currentValue={maxChunkSeconds}
            onChange={(v) => updateSettings({ maxChunkSeconds: v })}
          />
          <ButtonGroupSetting
            label="Silence Pause"
            description="How long silence must last before triggering a chunk split"
            options={SILENCE_PAUSE_OPTIONS}
            currentValue={silenceDurationMs}
            onChange={(v) => updateSettings({ silenceDurationMs: v })}
          />
          <ButtonGroupSetting
            label="Prompt Context"
            description="Characters of prior transcript fed to Whisper for continuity"
            options={PROMPT_CONTEXT_OPTIONS}
            currentValue={promptContextChars}
            onChange={(v) => updateSettings({ promptContextChars: v })}
          />
          <ButtonGroupSetting
            label="Prompt Decay"
            description="Clear prompt context after this much silence to prevent hallucination"
            options={PROMPT_DECAY_OPTIONS}
            currentValue={promptDecaySilenceSeconds}
            onChange={(v) => updateSettings({ promptDecaySilenceSeconds: v })}
          />
        </CollapsibleContent>
      </Collapsible>
    </>
  );
}
