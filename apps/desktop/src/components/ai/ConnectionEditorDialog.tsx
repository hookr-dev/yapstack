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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  AlertTriangle,
  Eye,
  EyeOff,
  ExternalLink,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { toast } from "sonner";
import {
  fetchCustomModels,
  shouldAutoRefreshModels,
  type AIProviderKind,
  type Connection,
} from "@/lib/ai";
import { CustomBaseUrlField } from "./CustomProviderFields";
import { useRefreshConnectionModels } from "@/hooks/useRefreshConnectionModels";

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

  // Local model-catalog state. Seeded from `initial` on open. Mutated by
  // explicit refresh in the dialog; committed to the Connection on save.
  const [localModels, setLocalModels] = useState<string[] | undefined>(undefined);
  const [localFetchedAt, setLocalFetchedAt] = useState<string | undefined>(undefined);
  const [localFetchError, setLocalFetchError] = useState<string | undefined>(undefined);
  const [fetching, setFetching] = useState(false);

  // Baselines for "has the user changed something that invalidates the cached
  // model catalog?" Seeded when the dialog opens and moved forward by a manual
  // Refresh (which re-validates the current values). Drives the background
  // refresh-on-save decision in handleSubmit.
  const [baseUrlAtOpen, setBaseUrlAtOpen] = useState("");
  const [apiKeyAtOpen, setApiKeyAtOpen] = useState("");

  const { refresh: refreshPersistedConnection } = useRefreshConnectionModels();

  useEffect(() => {
    if (!open) return;
    setName(initial?.name ?? "");
    setKind(initial?.kind ?? "openai");
    setApiKey(initial?.apiKey ?? "");
    const seedBaseUrl = initial?.baseUrl ?? DEFAULT_BASE_URLS[initial?.kind ?? "openai"];
    setBaseUrl(seedBaseUrl);
    setBaseUrlAtOpen(seedBaseUrl);
    setApiKeyAtOpen(initial?.apiKey ?? "");
    setShowKey(false);
    setLocalModels(initial?.availableModels);
    setLocalFetchedAt(initial?.fetchedAt);
    setLocalFetchError(initial?.fetchError);
    setFetching(false);
  }, [open, initial]);

  function handleKindChange(next: AIProviderKind) {
    setKind(next);
    setBaseUrl(DEFAULT_BASE_URLS[next]);
  }

  async function handleRefresh() {
    if (!baseUrl.trim()) return;
    setFetching(true);
    setLocalFetchError(undefined);
    try {
      const models = await fetchCustomModels(baseUrl, apiKey);
      setLocalModels(models);
      setLocalFetchedAt(new Date().toISOString());
      // We just validated the current endpoint + key, so move the baselines
      // forward. Saving now won't fire a redundant background re-fetch.
      setBaseUrlAtOpen(baseUrl);
      setApiKeyAtOpen(apiKey);
      if (models.length === 0) {
        toast.warning(
          "Server reported no models. You can still type a model name manually in a Profile.",
        );
      } else {
        toast.success(`Cached ${models.length} model${models.length === 1 ? "" : "s"}.`);
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setLocalFetchError(message);
      toast.error(`Couldn't fetch models: ${message}`);
    } finally {
      setFetching(false);
    }
  }

  function handleSubmit() {
    const trimmedName = name.trim() || KIND_LABELS[kind].split(" ")[0]!;
    const id = initial?.id ?? crypto.randomUUID();

    const next: Connection = {
      id,
      name: trimmedName,
      kind,
      baseUrl,
      apiKey: apiKey.trim(),
      ...(localModels !== undefined && { availableModels: localModels }),
      ...(localFetchedAt !== undefined && { fetchedAt: localFetchedAt }),
      ...(localFetchError !== undefined && { fetchError: localFetchError }),
    };
    onSubmit(next);
    onOpenChange(false);

    // Background refresh: on create with no fetched catalog, OR on edit when
    // the endpoint or the API key changed since the dialog opened (a corrected
    // key must re-validate too — otherwise a stale fetchError sticks). A manual
    // Refresh moves the baselines forward, so this won't double-fetch.
    // Fire-and-forget so the dialog closes immediately; the hook toasts on done.
    const shouldAutoRefresh = shouldAutoRefreshModels({
      mode,
      hasLocalModels: localModels !== undefined,
      fetching,
      baseUrl,
      baseUrlBaseline: baseUrlAtOpen,
      apiKey,
      apiKeyBaseline: apiKeyAtOpen,
    });
    if (shouldAutoRefresh) {
      // Defer one tick so the store update lands before the hook reads it.
      queueMicrotask(() => {
        refreshPersistedConnection(id);
      });
    }
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

          {kind === "custom" ? (
            <CustomBaseUrlField baseUrl={baseUrl} onChange={setBaseUrl} />
          ) : (
            <div className="space-y-2">
              <Label className="text-xs text-muted-foreground">Base URL</Label>
              <Input
                value={baseUrl}
                readOnly
                className="h-8 text-xs"
              />
            </div>
          )}

          <ModelsPanel
            fetching={fetching}
            localModels={localModels}
            localFetchError={localFetchError}
            onRefresh={handleRefresh}
            disabled={!baseUrl.trim()}
          />
        </div>

        <DialogFooter>
          <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button size="sm" onClick={handleSubmit} disabled={!canSubmit}>
            {mode === "create" ? "Add Connection" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ModelsPanel({
  fetching,
  localModels,
  localFetchError,
  onRefresh,
  disabled,
}: {
  fetching: boolean;
  localModels?: string[];
  localFetchError?: string;
  onRefresh: () => void;
  disabled: boolean;
}) {
  const count = localModels?.length;
  const buttonLabel = localModels === undefined ? "Fetch models" : "Refresh";

  return (
    <div className="space-y-2">
      <Label className="text-xs text-muted-foreground">Models</Label>
      <div className="flex items-center justify-between gap-3 rounded-md border border-border bg-muted/30 px-3 py-2">
        <ModelsPanelStatus
          fetching={fetching}
          fetchError={localFetchError}
          count={count}
        />
        <Button
          variant="outline"
          size="sm"
          onClick={onRefresh}
          disabled={fetching || disabled}
          className="shrink-0 text-xs"
        >
          {fetching ? (
            <Loader2 className="mr-1.5 h-3 w-3 animate-spin" />
          ) : (
            <RefreshCw className="mr-1.5 h-3 w-3" />
          )}
          {buttonLabel}
        </Button>
      </div>
    </div>
  );
}

function ModelsPanelStatus({
  fetching,
  fetchError,
  count,
}: {
  fetching: boolean;
  fetchError?: string;
  count: number | undefined;
}) {
  if (fetching) {
    return (
      <span className="text-[11px] text-muted-foreground">
        Fetching from server…
      </span>
    );
  }
  if (fetchError) {
    return (
      <span className="flex min-w-0 items-center gap-1 text-[11px] text-destructive">
        <AlertTriangle className="h-3 w-3 shrink-0" />
        <span className="truncate" title={fetchError}>
          {fetchError}
        </span>
      </span>
    );
  }
  if (count === undefined) {
    return (
      <span className="text-[11px] text-muted-foreground">
        No models fetched yet.
      </span>
    );
  }
  if (count === 0) {
    return (
      <span className="text-[11px] text-muted-foreground">
        Server reported no models.
      </span>
    );
  }
  return (
    <span className="text-[11px] text-muted-foreground">{count} cached</span>
  );
}
