import { useState, useEffect } from "react";
import { useAppStore } from "@/stores/appStore";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { testConnection, getModelsForProvider, getAllModelsGrouped } from "@/lib/ai";
import { DEFAULT_AI_SETTINGS } from "@/lib/ai";
import type { AIProvider, AISettings } from "@/lib/ai";
import { Eye, EyeOff, ExternalLink, Loader2 } from "lucide-react";
import { trackAIProviderChanged, trackAIConnectionTested } from "@/lib/analytics";

const PROVIDER_LABELS: Record<AIProvider, string> = {
  openai: "OpenAI",
  openrouter: "OpenRouter",
  custom: "Custom",
};

const MODEL_PLACEHOLDERS: Record<AIProvider, string> = {
  openai: "gpt-4o-mini",
  openrouter: "anthropic/claude-sonnet-4",
  custom: "model-name",
};

export function AITab() {
  const ai = useAppStore((s) => s.settings.ai) ?? DEFAULT_AI_SETTINGS;
  const updateSettings = useAppStore((s) => s.updateSettings);
  const [showKey, setShowKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{
    ok: boolean;
    error?: string;
  } | null>(null);
  const provider = ai.activeProvider;
  const config = ai.providers[provider];
  const models = getModelsForProvider(provider);
  const isKnownModel = models?.some((m) => m.id === config.model) ?? false;
  const [customMode, setCustomMode] = useState(!isKnownModel && !!models);

  // Reset customMode when provider changes
  useEffect(() => {
    const m = getModelsForProvider(provider);
    const known = m?.some((opt) => opt.id === config.model) ?? false;
    setCustomMode(!known && !!m);
  }, [provider]); // eslint-disable-line react-hooks/exhaustive-deps

  function updateAI(partial: Partial<AISettings>) {
    updateSettings({ ai: { ...ai, ...partial } });
  }

  function updateProviderConfig(field: string, value: string) {
    updateSettings({
      ai: {
        ...ai,
        providers: {
          ...ai.providers,
          [provider]: { ...config, [field]: value },
        },
      },
    });
  }

  async function handleTestConnection() {
    setTesting(true);
    setTestResult(null);
    const result = await testConnection(ai);
    trackAIConnectionTested({ provider, success: result.ok ? 1 : 0 });
    setTestResult(result);
    setTesting(false);
  }

  return (
    <>
      {/* Provider */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Provider</Label>
        <Select
          value={provider}
          onValueChange={(v) => {
            updateAI({ activeProvider: v as AIProvider });
            trackAIProviderChanged({ provider: v });
          }}
        >
          <SelectTrigger className="h-8 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(Object.keys(PROVIDER_LABELS) as AIProvider[]).map((p) => (
              <SelectItem key={p} value={p} className="text-xs">
                {PROVIDER_LABELS[p]}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* API Key */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">API Key</Label>
        <div>
          <div className="relative">
            <Input
              type={showKey ? "text" : "password"}
              value={config.apiKey}
              onChange={(e) => updateProviderConfig("apiKey", e.target.value)}
              placeholder="sk-..."
              className="h-8 text-xs pr-8"
            />
            <button
              type="button"
              className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              onClick={() => setShowKey(!showKey)}
            >
              {showKey ? (
                <EyeOff className="h-3.5 w-3.5" />
              ) : (
                <Eye className="h-3.5 w-3.5" />
              )}
            </button>
          </div>
          {(provider === "openai" || provider === "openrouter") && (
            <Button variant="link" size="inline" className="mt-1" asChild>
              <a
                href={
                  provider === "openai"
                    ? "https://platform.openai.com/api-keys"
                    : "https://openrouter.ai/settings/keys"
                }
                target="_blank"
                rel="noopener noreferrer"
              >
                Get your {provider === "openai" ? "OpenAI" : "OpenRouter"} API key
                <ExternalLink />
              </a>
            </Button>
          )}
        </div>
      </div>

      {/* Model */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Model</Label>
        {models ? (
          <>
            <Select
              value={customMode ? "__custom" : config.model}
              onValueChange={(v) => {
                if (v === "__custom") {
                  setCustomMode(true);
                } else {
                  setCustomMode(false);
                  updateProviderConfig("model", v);
                }
              }}
            >
              <SelectTrigger className="h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {getAllModelsGrouped(provider).map((group) => (
                  <SelectGroup key={group.provider}>
                    <SelectLabel className="text-[9px] text-muted-foreground/50 uppercase">
                      {group.providerLabel}
                    </SelectLabel>
                    {group.models.map((m) => (
                      <SelectItem
                        key={`${group.provider}:${m.id}`}
                        value={m.id}
                        className="text-xs"
                        disabled={!m.available}
                      >
                        <span className="flex items-center gap-2">
                          {m.label}
                          {m.recommended && m.available && (
                            <Badge variant="secondary" className="text-[9px] px-1 py-0">
                              Recommended
                            </Badge>
                          )}
                        </span>
                      </SelectItem>
                    ))}
                  </SelectGroup>
                ))}
                <SelectItem value="__custom" className="text-xs">
                  Custom...
                </SelectItem>
              </SelectContent>
            </Select>
            {customMode && (
              <Input
                value={config.model}
                onChange={(e) => updateProviderConfig("model", e.target.value)}
                placeholder={MODEL_PLACEHOLDERS[provider]}
                className="h-8 text-xs"
              />
            )}
          </>
        ) : (
          <Input
            value={config.model}
            onChange={(e) => updateProviderConfig("model", e.target.value)}
            placeholder={MODEL_PLACEHOLDERS[provider]}
            className="h-8 text-xs"
          />
        )}
      </div>

      {/* Base URL */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Base URL</Label>
        <Input
          value={config.baseUrl}
          onChange={(e) => updateProviderConfig("baseUrl", e.target.value)}
          readOnly={provider !== "custom"}
          className="h-8 text-xs"
        />
      </div>

      {/* Test Connection */}
      <div className="space-y-2">
        <Button
          size="sm"
          variant="outline"
          className="w-full text-xs"
          onClick={handleTestConnection}
          disabled={testing || !config.apiKey}
        >
          {testing && <Loader2 className="mr-1.5 h-3 w-3 animate-spin" />}
          Test Connection
        </Button>
        {testResult && (
          <Badge
            variant={testResult.ok ? "default" : "destructive"}
            className="text-[10px] w-full justify-center"
          >
            {testResult.ok
              ? "Connected successfully"
              : testResult.error || "Connection failed"}
          </Badge>
        )}
      </div>
    </>
  );
}
