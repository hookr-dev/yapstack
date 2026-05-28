/**
 * AIConfig builders and migration logic.
 *
 * Pure functions only — no I/O, no Zustand, no Tauri. Designed to be
 * exercised in unit tests and called from the persist `migrate` block
 * in stores/appStore.ts.
 */
import type {
  AIConfig,
  Connection,
  Profile,
} from "./ai";

/**
 * Legacy AI settings shape — used only by the v25 migration to read the
 * pre-refactor `state.settings.ai` value. Kept here (not in lib/ai.ts) so
 * the live AI surface area doesn't grow this dead type back. The default
 * baseUrl is inlined to support the "custom configured if baseUrl
 * differs from default" heuristic in isProviderConfigured.
 */
type LegacyAIProvider = "openai" | "openrouter" | "custom";

interface LegacyAIProviderConfig {
  apiKey: string;
  model: string;
  baseUrl: string;
  fetchedModels?: string[];
}

export interface LegacyAISettings {
  activeProvider: LegacyAIProvider;
  providers: Record<LegacyAIProvider, LegacyAIProviderConfig>;
}

const LEGACY_CUSTOM_DEFAULT_BASE_URL = "http://127.0.0.1:8080/v1";

/**
 * Pre-Commit-4 DictationSlot shape (carried `aiEnabled: boolean`, no
 * `profileId`). Mirrored here as a local interface so the migration is
 * decoupled from the live `DictationSlot` type in stores/appStore.ts —
 * that type changes in Commit 4 once the migration lands.
 */
export interface LegacyDictationSlot {
  id: string;
  name: string;
  enabled: boolean;
  aiEnabled: boolean;
  prompt: string;
  outputAction: string;
  defaultBinding?: string;
}

/**
 * Post-Commit-4 DictationSlot shape (`aiEnabled` dropped, `profileId`
 * added). Local mirror — see comment on LegacyDictationSlot.
 */
export interface MigratedDictationSlot {
  id: string;
  name: string;
  enabled: boolean;
  profileId: string | null;
  prompt: string;
  outputAction: string;
  defaultBinding?: string;
}

export interface MigrationResult {
  config: AIConfig;
  updatedSlots: MigratedDictationSlot[];
}

const KIND_DISPLAY: Record<LegacyAIProvider, string> = {
  openai: "OpenAI",
  openrouter: "OpenRouter",
  custom: "Custom",
};

function isProviderConfigured(
  kind: LegacyAIProvider,
  cfg: LegacyAIProviderConfig,
): boolean {
  if (kind === "custom") {
    // Custom is considered configured if either the apiKey is set OR the
    // user has changed baseUrl from the default — covers local servers
    // that don't require a key (llama.cpp, LM Studio).
    return cfg.apiKey !== "" || cfg.baseUrl !== LEGACY_CUSTOM_DEFAULT_BASE_URL;
  }
  return cfg.apiKey !== "";
}

/**
 * Transform legacy single-active-provider AISettings into the new AIConfig
 * (multiple Connections + Profiles + Assignments).
 *
 * Per locked design decision #6: only the *active* provider becomes a
 * Profile. Non-active but configured providers migrate as Connections so
 * their keys aren't silently lost, but no Profile is emitted for them —
 * users re-create Profiles in the new UI if they want them.
 *
 * Per locked design decision #12: dictation slots with `aiEnabled === true`
 * get `profileId = activeProfileId`; slots with `aiEnabled === false` get
 * `profileId = null`. `aiEnabled` is dropped from the output shape.
 */
export function migrateLegacyAISettings(
  legacy: LegacyAISettings,
  slots: LegacyDictationSlot[],
): MigrationResult {
  const connections: Connection[] = [];
  const profiles: Profile[] = [];
  let activeProfileId: string | null = null;

  for (const kind of Object.keys(legacy.providers) as LegacyAIProvider[]) {
    const cfg = legacy.providers[kind];
    if (!isProviderConfigured(kind, cfg)) continue;

    const connectionId = crypto.randomUUID();
    const connection: Connection = {
      id: connectionId,
      name: KIND_DISPLAY[kind],
      kind,
      baseUrl: cfg.baseUrl,
      apiKey: cfg.apiKey,
    };
    if (cfg.fetchedModels && cfg.fetchedModels.length > 0) {
      connection.availableModels = cfg.fetchedModels;
    }
    connections.push(connection);

    // Only emit a Profile for the active provider when it actually has a
    // model. A "custom" provider counts as configured on baseUrl alone (local
    // servers may need no key), so its model can be empty — emitting a Profile
    // with model "" would assign Chat/AI-actions a broken default on upgrade.
    // The Connection still migrates (key preserved); the user picks a model in
    // the new UI, leaving assignments null (greenfield) until then.
    if (kind === legacy.activeProvider && cfg.model.trim() !== "") {
      const profileId = crypto.randomUUID();
      profiles.push({
        id: profileId,
        name: `${KIND_DISPLAY[kind]} default`,
        connectionId,
        model: cfg.model,
      });
      activeProfileId = profileId;
    }
  }

  const config: AIConfig = {
    connections,
    profiles,
    assignments: {
      chatProfileId: activeProfileId,
      aiActionsProfileId: activeProfileId,
    },
  };

  const updatedSlots: MigratedDictationSlot[] = slots.map((slot) => ({
    id: slot.id,
    name: slot.name,
    enabled: slot.enabled,
    profileId: slot.aiEnabled && activeProfileId ? activeProfileId : null,
    prompt: slot.prompt,
    outputAction: slot.outputAction,
    defaultBinding: slot.defaultBinding,
  }));

  return { config, updatedSlots };
}

/**
 * Resolve a profileId to the concrete (Connection, model) pair.
 * Returns null when:
 *   - profileId is null
 *   - the Profile doesn't exist (deleted, never created, stale reference)
 *   - the referenced Connection doesn't exist (cascade-delete race, manual edit)
 *
 * Callers decide whether to surface an error or fall back to a default —
 * per locked design decision #8, AI feature consumers throw rather than
 * silently retrying on a different Profile.
 */
export function resolveProfile(
  config: AIConfig,
  profileId: string | null,
): { connection: Connection; model: string } | null {
  if (!profileId) return null;
  const profile = config.profiles.find((p) => p.id === profileId);
  if (!profile) return null;
  const connection = config.connections.find((c) => c.id === profile.connectionId);
  if (!connection) return null;
  return { connection, model: profile.model };
}
