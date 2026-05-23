import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Loader2 } from "lucide-react";
import { fetchCustomModels } from "@/lib/ai";

const CUSTOM_URL_PRESETS: { label: string; url: string }[] = [
  { label: "llama.cpp", url: "http://127.0.0.1:8080/v1" },
  { label: "LM Studio", url: "http://127.0.0.1:1234/v1" },
  { label: "Ollama", url: "http://127.0.0.1:11434/v1" },
  { label: "vLLM", url: "http://127.0.0.1:8000/v1" },
];

export function CustomBaseUrlField({
  baseUrl,
  onChange,
}: {
  baseUrl: string;
  onChange: (next: string) => void;
}) {
  return (
    <div className="space-y-2">
      <Label className="text-xs text-muted-foreground">Base URL</Label>
      <Input
        value={baseUrl}
        onChange={(e) => onChange(e.target.value)}
        placeholder="http://127.0.0.1:8080/v1"
        className="h-8 text-xs"
      />
      <div className="flex flex-wrap gap-1">
        {CUSTOM_URL_PRESETS.map((p) => (
          <button
            key={p.url}
            type="button"
            onClick={() => onChange(p.url)}
            className={`text-[10px] px-2 py-0.5 rounded border transition-colors ${
              baseUrl === p.url
                ? "bg-muted border-border text-foreground"
                : "border-border/50 text-muted-foreground hover:bg-muted hover:text-foreground"
            }`}
          >
            {p.label}
          </button>
        ))}
      </div>
    </div>
  );
}

export function CustomModelField({
  baseUrl,
  apiKey,
  model,
  fetchedModels,
  onModelChange,
  onFetchedModelsChange,
}: {
  baseUrl: string;
  apiKey?: string;
  model: string;
  fetchedModels?: string[];
  onModelChange: (next: string) => void;
  onFetchedModelsChange: (next: string[]) => void;
}) {
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleFetch() {
    setFetching(true);
    setError(null);
    try {
      const ids = await fetchCustomModels(baseUrl, apiKey);
      onFetchedModelsChange(ids);
      if (ids.length > 0 && !ids.includes(model)) {
        onModelChange(ids[0]!);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setFetching(false);
  }

  return (
    <div className="space-y-2">
      <Label className="text-xs text-muted-foreground">Model</Label>
      <Button
        size="sm"
        variant="outline"
        className="w-full text-xs"
        onClick={handleFetch}
        disabled={fetching || !baseUrl}
      >
        {fetching && <Loader2 className="mr-1.5 h-3 w-3 animate-spin" />}
        {fetchedModels ? "Refresh Models" : "Fetch Models from Server"}
      </Button>
      {fetchedModels && fetchedModels.length > 0 && (
        <Select
          value={fetchedModels.includes(model) ? model : ""}
          onValueChange={(v) => onModelChange(v)}
        >
          <SelectTrigger className="h-8 w-full text-xs">
            <SelectValue placeholder="Pick a fetched model..." />
          </SelectTrigger>
          <SelectContent>
            {fetchedModels.map((id) => (
              <SelectItem key={id} value={id} className="text-xs">
                {id}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
      <Input
        value={model}
        onChange={(e) => onModelChange(e.target.value)}
        placeholder="model-name"
        className="h-8 text-xs"
      />
      {error && (
        <Badge variant="destructive" className="text-[10px] w-full justify-center">
          {error}
        </Badge>
      )}
      {fetchedModels && fetchedModels.length === 0 && !error && (
        <Badge variant="secondary" className="text-[10px] w-full justify-center">
          Server returned no models
        </Badge>
      )}
    </div>
  );
}
