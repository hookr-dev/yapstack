import { useEffect, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Eye, EyeOff, ExternalLink, Loader2 } from "lucide-react";
import { toast } from "sonner";
import {
  fetchCustomModels,
  type AIProviderKind,
  type Connection,
} from "@/lib/ai";

const KIND_LABELS: Record<AIProviderKind, string> = {
  openai: "OpenAI",
  openrouter: "OpenRouter",
  custom: "Custom (OpenAI-compatible)",
};

const DEFAULT_BASE_URLS: Record<AIProviderKind, string> = {
  openai: "https://api.openai.com/v1",
  openrouter: "https://openrouter.ai/api/v1",
  custom: "http://127.0.0.1:8080/v1",
};

const API_KEY_DOC_URLS: Partial<Record<AIProviderKind, { label: string; url: string }>> = {
  openai: {
    label: "Get your OpenAI API key",
    url: "https://platform.openai.com/api-keys",
  },
  openrouter: {
    label: "Get your OpenRouter API key",
    url: "https://openrouter.ai/settings/keys",
  },
};

export interface ConnectionEditorDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mode: "create" | "edit";
  initial?: Connection;
  onSubmit: (connection: Connection) => void;
}

export function ConnectionEditorDialog({
  open,
  onOpenChange,
  mode,
  initial,
  onSubmit,
}: ConnectionEditorDialogProps) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<AIProviderKind>("openai");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState(DEFAULT_BASE_URLS.openai);
  const [showKey, setShowKey] = useState(false);
  const [fetching, setFetching] = useState(false);

  // Re-seed local state whenever the dialog opens. Doing it on `open`
  // (not on `initial` identity) avoids tearing if the caller passes a
  // freshly-constructed object on every render.
  useEffect(() => {
    if (!open) return;
    setName(initial?.name ?? "");
    setKind(initial?.kind ?? "openai");
    setApiKey(initial?.apiKey ?? "");
    setBaseUrl(initial?.baseUrl ?? DEFAULT_BASE_URLS[initial?.kind ?? "openai"]);
    setShowKey(false);
  }, [open, initial]);

  function handleKindChange(next: AIProviderKind) {
    setKind(next);
    // Reset baseUrl to the kind's default when switching kinds, so a user
    // who picked Custom and typed a localhost URL doesn't carry it into
    // OpenAI by accident. For Custom we still seed the default rather
    // than leaving blank — most local servers run on 8080.
    setBaseUrl(DEFAULT_BASE_URLS[next]);
  }

  async function handleSubmit() {
    const trimmedName = name.trim() || KIND_LABELS[kind].split(" ")[0]!;
    const id = initial?.id ?? crypto.randomUUID();

    // Fetch models so the Profile picker has something to show. Failures
    // are non-blocking — the Connection still saves with `fetchError`
    // set, and the user can refresh from the edit dialog later or just
    // type a model name manually in the Profile editor.
    let availableModels: string[] | undefined;
    let fetchError: string | undefined;
    setFetching(true);
    try {
      availableModels = await fetchCustomModels(baseUrl);
      if (availableModels.length === 0) {
        // Empty list isn't a failure — some servers expose nothing on
        // /models. Keep the response so we don't re-fetch on every open,
        // and let the toast hint that the user may need to type manually.
        toast.warning(
          `Connected to ${trimmedName}, but the server reported no models. You can type a model name manually when creating a Profile.`,
        );
      }
    } catch (e) {
      fetchError = e instanceof Error ? e.message : String(e);
      toast.warning(
        `Saved ${trimmedName}, but couldn't fetch models: ${fetchError}. Refresh from the edit dialog once the server is reachable, or type a model name manually.`,
      );
    } finally {
      setFetching(false);
    }

    const next: Connection = {
      id,
      name: trimmedName,
      kind,
      baseUrl,
      apiKey: apiKey.trim(),
      ...(availableModels !== undefined && { availableModels }),
      ...(availableModels !== undefined && { fetchedAt: new Date().toISOString() }),
      ...(fetchError !== undefined && { fetchError }),
    };
    onSubmit(next);
    onOpenChange(false);
  }

  const apiKeyOptional = kind === "custom";
  const apiKeyDoc = API_KEY_DOC_URLS[kind];
  const canSubmit = !fetching && baseUrl.trim().length > 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {mode === "create" ? "Add Connection" : "Edit Connection"}
          </DialogTitle>
          {mode === "create" && (
            <DialogDescription>
              Connect an AI provider. Profiles you create later can target
              this connection.
            </DialogDescription>
          )}
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">Name</Label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={KIND_LABELS[kind]}
              className="h-8 text-xs"
              autoFocus
            />
          </div>

          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">Provider</Label>
            <Select
              value={kind}
              onValueChange={(v) => handleKindChange(v as AIProviderKind)}
            >
              <SelectTrigger className="h-8 w-full text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(Object.keys(KIND_LABELS) as AIProviderKind[]).map((k) => (
                  <SelectItem key={k} value={k} className="text-xs">
                    {KIND_LABELS[k]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">
              API Key
              {apiKeyOptional && (
                <span className="ml-1 text-muted-foreground/60">(optional)</span>
              )}
            </Label>
            <div>
              <div className="relative">
                <Input
                  type={showKey ? "text" : "password"}
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  placeholder={apiKeyOptional ? "Leave blank if not required" : "sk-..."}
                  className="h-8 text-xs pr-8"
                />
                <button
                  type="button"
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                  onClick={() => setShowKey((s) => !s)}
                >
                  {showKey ? (
                    <EyeOff className="h-3.5 w-3.5" />
                  ) : (
                    <Eye className="h-3.5 w-3.5" />
                  )}
                </button>
              </div>
              {apiKeyDoc && (
                <Button variant="link" size="inline" className="mt-1" asChild>
                  <a href={apiKeyDoc.url} target="_blank" rel="noopener noreferrer">
                    {apiKeyDoc.label}
                    <ExternalLink />
                  </a>
                </Button>
              )}
            </div>
          </div>

          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">Base URL</Label>
            <Input
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              readOnly={kind !== "custom"}
              className="h-8 text-xs"
            />
            {kind === "custom" && (
              <p className="text-[10px] text-muted-foreground leading-relaxed">
                Any OpenAI-compatible server (llama.cpp, LM Studio, Ollama, vLLM).
              </p>
            )}
          </div>

          {initial?.availableModels !== undefined && (
            <Badge variant="secondary" className="text-[10px]">
              {initial.availableModels.length} model
              {initial.availableModels.length === 1 ? "" : "s"} cached
            </Badge>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button size="sm" onClick={handleSubmit} disabled={!canSubmit}>
            {fetching && <Loader2 className="mr-1.5 h-3 w-3 animate-spin" />}
            {mode === "create" ? "Add Connection" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
