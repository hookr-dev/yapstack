import { useState, useEffect, Fragment } from "react";
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
import {
  testConnection,
  getModelsForProvider,
  getAllModelsGrouped,
} from "@/lib/ai";
import { DEFAULT_AI_SETTINGS } from "@/lib/ai";
import type { AIProvider, AIProviderConfig, AISettings } from "@/lib/ai";
import {
  CustomBaseUrlField,
  CustomModelField,
} from "@/components/ai/CustomProviderFields";
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

  // Reset per-provider UI state when provider changes
  useEffect(() => {
    const m = getModelsForProvider(provider);
    const known = m?.some((opt) => opt.id === config.model) ?? false;
    setCustomMode(!known && !!m);
    setTestResult(null);
  }, [provider]); // eslint-disable-line react-hooks/exhaustive-deps

  function updateAI(partial: Partial<AISettings>) {
    updateSettings({ ai: { ...ai, ...partial } });
  }

  function updateProviderConfig(field: string, value: string) {
    updateProviderConfigPartial({ [field]: value });
  }

  function updateProviderConfigPartial(updates: Partial<AIProviderConfig>) {
    updateSettings({
      ai: {
        ...ai,
        providers: {
          ...ai.providers,
          [provider]: { ...config, ...updates },
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

  const providerField = (
    <div className="space-y-2">
      <Label className="text-xs text-muted-foreground">Provider</Label>
      <Select
        value={provider}
        onValueChange={(v) => {
          updateAI({ activeProvider: v as AIProvider });
          trackAIProviderChanged({ provider: v });
        }}
      >
        <SelectTrigger className="h-8 w-full text-xs">
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
      {provider === "custom" && (
        <p className="text-[10px] text-muted-foreground leading-relaxed">
          Points at any OpenAI-compatible server (llama.cpp, LM Studio,
          Ollama). Tool actions (rename, save, pin) only fire on models with
          tool-calling support.
        </p>
      )}
    </div>
  );

  const apiKeyField = (
    <div className="space-y-2">
      <Label className="text-xs text-muted-foreground">
        API Key{provider === "custom" && (
          <span className="ml-1 text-muted-foreground/60">(optional)</span>
        )}
      </Label>
      <div>
        <div className="relative">
          <Input
            type={showKey ? "text" : "password"}
            value={config.apiKey}
            onChange={(e) => updateProviderConfig("apiKey", e.target.value)}
            placeholder={
              provider === "custom" ? "Leave blank if not required" : "sk-..."
            }
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
  );

  const baseUrlField =
    provider === "custom" ? (
      <CustomBaseUrlField config={config} onUpdate={updateProviderConfigPartial} />
    ) : (
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Base URL</Label>
        <Input
          value={config.baseUrl}
          readOnly
          placeholder="http://127.0.0.1:8080/v1"
          className="h-8 text-xs"
        />
      </div>
    );

  const knownModelField = models ? (
    <div className="space-y-2">
      <Label className="text-xs text-muted-foreground">Model</Label>
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
        <SelectTrigger className="h-8 w-full text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {getAllModelsGrouped(provider, config).map((group) => (
            <SelectGroup key={group.provider}>
              <SelectLabel className="text-[9px] text-muted-foreground/50 uppercase">
                {group.providerLabel}
              </SelectLabel>
              {group.models.map((m) => (
                <SelectItem
                  key={`${group.provider}:${m.id}`}
                  value={m.id}
                  className="text-xs"
                >
                  <span className="flex items-center gap-2">
                    {m.label}
                    {m.recommended && (
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
    </div>
  ) : null;

  const customModelField = (
    <CustomModelField config={config} onUpdate={updateProviderConfigPartial} />
  );

  const testButton = (
    <div className="space-y-2">
      <Button
        size="sm"
        variant="outline"
        className="w-full text-xs"
        onClick={handleTestConnection}
        disabled={testing || (provider !== "custom" && !config.apiKey)}
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
  );

  const modelField = provider === "custom" ? customModelField : knownModelField;
  const ordered = provider === "custom"
    ? [providerField, baseUrlField, apiKeyField, modelField, testButton]
    : [providerField, apiKeyField, modelField, baseUrlField, testButton];

  return <>{ordered.map((node, i) => <Fragment key={i}>{node}</Fragment>)}</>;
}
