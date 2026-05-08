import { useMemo, useState } from "react";
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
import { toast } from "sonner";
import type { EngineKindDto } from "@/lib/tauri";
import { ModelSection } from "./ModelSection";
import { ParakeetModelSection } from "./ParakeetModelSection";

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

/// Friendly display labels for the language codes the engine catalogue
/// returns. Codes not in this map fall back to displaying the raw code so
/// the dropdown stays usable as the catalogue grows.
const LANGUAGE_LABELS: Record<string, string> = {
  en: "English",
  es: "Spanish",
  fr: "French",
  de: "German",
  ja: "Japanese",
  zh: "Chinese",
  ko: "Korean",
  pt: "Portuguese",
  it: "Italian",
  ru: "Russian",
  nl: "Dutch",
  pl: "Polish",
  sv: "Swedish",
  no: "Norwegian",
  da: "Danish",
  fi: "Finnish",
  cs: "Czech",
  sk: "Slovak",
  hu: "Hungarian",
  ro: "Romanian",
  bg: "Bulgarian",
  hr: "Croatian",
  sl: "Slovenian",
  el: "Greek",
  uk: "Ukrainian",
  et: "Estonian",
  lv: "Latvian",
  lt: "Lithuanian",
  ga: "Irish",
  mt: "Maltese",
  ar: "Arabic",
  hi: "Hindi",
  vi: "Vietnamese",
  th: "Thai",
  he: "Hebrew",
  tr: "Turkish",
  id: "Indonesian",
  ca: "Catalan",
};

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
  const selectedEngine = useAppStore((s) => s.settings.selectedEngine);
  const engineCatalogue = useAppStore((s) => s.engineCatalogue);
  const maxChunkSeconds = useAppStore((s) => s.settings.maxChunkSeconds);
  const silenceDurationMs = useAppStore((s) => s.settings.silenceDurationMs);
  const promptContextChars = useAppStore((s) => s.settings.promptContextChars);
  const promptDecaySilenceSeconds = useAppStore(
    (s) => s.settings.promptDecaySilenceSeconds,
  );
  const sortformerStatus = useAppStore((s) => s.sortformerStatus);
  const modelDownloadProgress = useAppStore((s) => s.modelDownloadProgress);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const switchEngine = useAppStore((s) => s.switchEngine);
  const [showAdvanced, setShowAdvanced] = useState(false);

  // Derive available languages from the current engine's catalogue entry.
  // When the catalogue hasn't loaded yet, fall back to a permissive list
  // (the persisted language stays visible without flicker).
  const activeDescriptor = useMemo(
    () => engineCatalogue.find((d) => d.kind === selectedEngine),
    [engineCatalogue, selectedEngine],
  );
  const availableLanguages = useMemo(() => {
    const codes = activeDescriptor?.languages ?? [language];
    return codes.map((code) => ({
      value: code,
      label: LANGUAGE_LABELS[code] ?? code,
    }));
  }, [activeDescriptor, language]);

  // Both engines now diarize via the shared Sortformer post-pass; this is
  // kept in case a future engine opts out.
  const supportsDiarization = activeDescriptor?.supports_diarization ?? true;
  const sortformerDownloaded = sortformerStatus?.downloaded ?? false;
  const isDownloadingSortformer =
    modelDownloadProgress !== null && !sortformerDownloaded;

  const handleEngineChange = async (engine: EngineKindDto) => {
    if (engine === selectedEngine) return;
    try {
      await switchEngine(engine);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const handleLanguageChange = (code: string) => {
    updateSettings({ language: code });
  };

  // Clamp language when the engine changes — if the persisted code is no
  // longer in the active catalogue, fall back to the engine's primary code.
  const languageInList = availableLanguages.some((l) => l.value === language);
  const effectiveLanguage = languageInList
    ? language
    : availableLanguages[0]?.value ?? language;

  return (
    <>
      {/* Engine — Parakeet is the recommended default. The chip shows on
          the non-active engine so we nudge toward Parakeet without shouting
          at users who already picked it. */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">
          Transcription engine
        </Label>
        <div className="flex gap-1.5">
          {(["Parakeet", "Whisper"] as const).map((kind) => (
            <Button
              key={kind}
              size="sm"
              variant={selectedEngine === kind ? "default" : "outline"}
              className="relative flex-1 text-xs"
              onClick={() => handleEngineChange(kind)}
            >
              {kind}
              {kind === "Parakeet" && selectedEngine !== "Parakeet" && (
                <span className="ml-1.5 rounded-full bg-primary/15 px-1.5 py-0.5 text-[9px] font-medium text-primary">
                  Recommended
                </span>
              )}
            </Button>
          ))}
        </div>
        <p className="text-[10px] text-muted-foreground/60">
          {selectedEngine === "Parakeet"
            ? "NVIDIA Parakeet TDT v3 — fastest and most accurate for English and 24 other European languages."
            : "OpenAI Whisper — broader language coverage (99 languages) when Parakeet's language set isn't enough."}
        </p>
      </div>

      <Separator />

      {/* Language */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Language</Label>
        <Select value={effectiveLanguage} onValueChange={handleLanguageChange}>
          <SelectTrigger className="h-8 w-full text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent className="max-h-72">
            {availableLanguages.map((lang) => (
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

      {/* Model — depends on selected engine */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Model</Label>
        {selectedEngine === "Whisper" ? <ModelSection /> : <ParakeetModelSection />}
      </div>

      <Separator />

      {/* Speaker diarization is permanently on for both Whisper and
          Parakeet. The model lifecycle (download + delete) lives here
          so users can see what's running and recover storage if needed. */}
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <Label className="text-xs text-muted-foreground">
            Speaker diarization
          </Label>
          <span className="text-[10px] font-medium">
            {!supportsDiarization
              ? "Unavailable"
              : isDownloadingSortformer
                ? `Downloading ${Math.round(modelDownloadProgress ?? 0)}%`
                : sortformerDownloaded
                  ? "Active"
                  : "Will download on next session"}
          </span>
        </div>
        <p className="text-[10px] text-muted-foreground/60">
          {supportsDiarization
            ? "Identifies distinct speakers in your recordings using NVIDIA Sortformer (~50 MB). Always on; dictation skips it automatically."
            : "The selected engine doesn't support speaker diarization."}
        </p>
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
            description="Characters of prior transcript fed to Whisper for continuity (Whisper only)"
            options={PROMPT_CONTEXT_OPTIONS}
            currentValue={promptContextChars}
            onChange={(v) => updateSettings({ promptContextChars: v })}
          />
          <ButtonGroupSetting
            label="Prompt Decay"
            description="Clear prompt context after this much silence to prevent hallucination (Whisper only)"
            options={PROMPT_DECAY_OPTIONS}
            currentValue={promptDecaySilenceSeconds}
            onChange={(v) => updateSettings({ promptDecaySilenceSeconds: v })}
          />
        </CollapsibleContent>
      </Collapsible>
    </>
  );
}
