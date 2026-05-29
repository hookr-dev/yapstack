import { describe, it, expect } from "vitest";
import {
  migrateLegacyAISettings,
  resolveProfile,
  type LegacyAISettings,
  type LegacyDictationSlot,
} from "./ai-config";
import type { AIConfig } from "./ai";

// Mirrors the pre-refactor default for the `custom` provider so the test
// fixture matches what the migration's isProviderConfigured() check expects.
const LEGACY_CUSTOM_DEFAULT_BASE_URL = "http://127.0.0.1:8080/v1";

function legacy(overrides: Partial<LegacyAISettings> = {}): LegacyAISettings {
  return {
    activeProvider: "openai",
    providers: {
      openai: { apiKey: "", model: "gpt-5.4-mini", baseUrl: "https://api.openai.com/v1" },
      openrouter: {
        apiKey: "",
        model: "anthropic/claude-haiku-4.5",
        baseUrl: "https://openrouter.ai/api/v1",
      },
      custom: {
        apiKey: "",
        model: "",
        baseUrl: LEGACY_CUSTOM_DEFAULT_BASE_URL,
      },
    },
    ...overrides,
  };
}

function slot(overrides: Partial<LegacyDictationSlot> = {}): LegacyDictationSlot {
  return {
    id: "slot-1",
    name: "Raw Dictation",
    enabled: true,
    aiEnabled: false,
    prompt: "",
    outputAction: "paste",
    ...overrides,
  };
}

describe("migrateLegacyAISettings", () => {
  describe("Connection emission", () => {
    it("emits no Connections when all providers are unconfigured", () => {
      const { config } = migrateLegacyAISettings(legacy(), []);
      expect(config.connections).toEqual([]);
      expect(config.profiles).toEqual([]);
      expect(config.assignments).toEqual({
        chatProfileId: null,
        aiActionsProfileId: null,
      });
    });

    it("emits one Connection when only the active provider is configured", () => {
      const { config } = migrateLegacyAISettings(
        legacy({
          providers: {
            ...legacy().providers,
            openai: {
              apiKey: "sk-test",
              model: "gpt-5.4-mini",
              baseUrl: "https://api.openai.com/v1",
            },
          },
        }),
        [],
      );
      expect(config.connections).toHaveLength(1);
      expect(config.connections[0]).toMatchObject({
        name: "OpenAI",
        kind: "openai",
        apiKey: "sk-test",
        baseUrl: "https://api.openai.com/v1",
      });
    });

    it("emits Connections for all configured providers, even non-active ones", () => {
      const { config } = migrateLegacyAISettings(
        legacy({
          providers: {
            openai: {
              apiKey: "sk-openai",
              model: "gpt-5.4-mini",
              baseUrl: "https://api.openai.com/v1",
            },
            openrouter: {
              apiKey: "sk-router",
              model: "anthropic/claude-haiku-4.5",
              baseUrl: "https://openrouter.ai/api/v1",
            },
            custom: {
              apiKey: "",
              model: "",
              baseUrl: LEGACY_CUSTOM_DEFAULT_BASE_URL,
            },
          },
        }),
        [],
      );
      expect(config.connections).toHaveLength(2);
      const kinds = config.connections.map((c) => c.kind).sort();
      expect(kinds).toEqual(["openai", "openrouter"]);
    });

    it("treats custom with a changed baseUrl (no apiKey) as configured", () => {
      const { config } = migrateLegacyAISettings(
        legacy({
          providers: {
            ...legacy().providers,
            custom: {
              apiKey: "",
              model: "llama-3.1-8b",
              baseUrl: "http://localhost:11434/v1",
            },
          },
        }),
        [],
      );
      expect(config.connections).toHaveLength(1);
      expect(config.connections[0]).toMatchObject({
        kind: "custom",
        baseUrl: "http://localhost:11434/v1",
      });
    });

    it("preserves fetchedModels as availableModels on Connection", () => {
      const { config } = migrateLegacyAISettings(
        legacy({
          providers: {
            ...legacy().providers,
            custom: {
              apiKey: "",
              model: "llama-3.1-8b",
              baseUrl: "http://localhost:11434/v1",
              fetchedModels: ["llama-3.1-8b", "qwen-2.5-32b"],
            },
          },
        }),
        [],
      );
      expect(config.connections[0]?.availableModels).toEqual([
        "llama-3.1-8b",
        "qwen-2.5-32b",
      ]);
    });
  });

  describe("Profile emission (active-only per decision #6)", () => {
    it("emits one Profile pointing at the active provider's Connection", () => {
      const { config } = migrateLegacyAISettings(
        legacy({
          activeProvider: "openai",
          providers: {
            ...legacy().providers,
            openai: {
              apiKey: "sk-openai",
              model: "gpt-5.4-mini",
              baseUrl: "https://api.openai.com/v1",
            },
            openrouter: {
              apiKey: "sk-router",
              model: "anthropic/claude-haiku-4.5",
              baseUrl: "https://openrouter.ai/api/v1",
            },
          },
        }),
        [],
      );
      expect(config.profiles).toHaveLength(1);
      const openaiConnection = config.connections.find((c) => c.kind === "openai");
      expect(config.profiles[0]).toMatchObject({
        connectionId: openaiConnection!.id,
        model: "gpt-5.4-mini",
      });
      expect(config.assignments.chatProfileId).toBe(config.profiles[0]!.id);
      expect(config.assignments.aiActionsProfileId).toBe(config.profiles[0]!.id);
    });

    it("emits no Profile when the active provider is not configured", () => {
      // openai is the active provider but has no apiKey;
      // openrouter is configured but not active.
      const { config } = migrateLegacyAISettings(
        legacy({
          activeProvider: "openai",
          providers: {
            ...legacy().providers,
            openrouter: {
              apiKey: "sk-router",
              model: "anthropic/claude-haiku-4.5",
              baseUrl: "https://openrouter.ai/api/v1",
            },
          },
        }),
        [],
      );
      expect(config.connections).toHaveLength(1);
      expect(config.connections[0]?.kind).toBe("openrouter");
      expect(config.profiles).toEqual([]);
      expect(config.assignments).toEqual({
        chatProfileId: null,
        aiActionsProfileId: null,
      });
    });

    it("emits no Profile when the active custom provider has an empty model", () => {
      // Custom counts as configured on baseUrl alone, but with no model a
      // Profile would assign Chat/AI-actions a blank, broken model on upgrade.
      const { config } = migrateLegacyAISettings(
        legacy({
          activeProvider: "custom",
          providers: {
            ...legacy().providers,
            custom: {
              apiKey: "",
              model: "",
              baseUrl: "http://localhost:11434/v1",
            },
          },
        }),
        [],
      );
      // Connection still migrates so the endpoint isn't lost...
      expect(config.connections).toHaveLength(1);
      expect(config.connections[0]?.kind).toBe("custom");
      // ...but no Profile / assignment is created.
      expect(config.profiles).toEqual([]);
      expect(config.assignments).toEqual({
        chatProfileId: null,
        aiActionsProfileId: null,
      });
    });
  });

  describe("DictationSlot migration (decision #12)", () => {
    it("preserves slot fields except aiEnabled, adds profileId", () => {
      const { config, updatedSlots } = migrateLegacyAISettings(
        legacy({
          providers: {
            ...legacy().providers,
            openai: {
              apiKey: "sk-test",
              model: "gpt-5.4-mini",
              baseUrl: "https://api.openai.com/v1",
            },
          },
        }),
        [
          slot({
            id: "raw",
            name: "Raw",
            enabled: true,
            aiEnabled: false,
            prompt: "",
            outputAction: "paste",
            defaultBinding: "Cmd+Shift+R",
          }),
        ],
      );
      expect(updatedSlots).toHaveLength(1);
      expect(updatedSlots[0]).toEqual({
        id: "raw",
        name: "Raw",
        enabled: true,
        profileId: null,
        prompt: "",
        outputAction: "paste",
        defaultBinding: "Cmd+Shift+R",
      });
      // Sanity: no stray `aiEnabled` key.
      expect("aiEnabled" in updatedSlots[0]!).toBe(false);
      // Sanity: an active profile was emitted.
      expect(config.profiles).toHaveLength(1);
    });

    it("aiEnabled=true slots get the active profileId when one exists", () => {
      const { config, updatedSlots } = migrateLegacyAISettings(
        legacy({
          providers: {
            ...legacy().providers,
            openai: {
              apiKey: "sk-test",
              model: "gpt-5.4-mini",
              baseUrl: "https://api.openai.com/v1",
            },
          },
        }),
        [
          slot({ id: "clean", aiEnabled: true, prompt: "clean it up" }),
          slot({ id: "raw", aiEnabled: false, prompt: "" }),
        ],
      );
      const activeProfileId = config.profiles[0]!.id;
      expect(updatedSlots.find((s) => s.id === "clean")?.profileId).toBe(activeProfileId);
      expect(updatedSlots.find((s) => s.id === "raw")?.profileId).toBe(null);
    });

    it("aiEnabled=true slots become profileId=null when no active profile exists", () => {
      // No providers configured → no active profile → aiEnabled slots fall to null
      const { config, updatedSlots } = migrateLegacyAISettings(
        legacy(),
        [slot({ id: "clean", aiEnabled: true, prompt: "clean" })],
      );
      expect(config.profiles).toEqual([]);
      expect(updatedSlots[0]?.profileId).toBe(null);
    });
  });
});

describe("resolveProfile", () => {
  const config: AIConfig = {
    connections: [
      {
        id: "conn-1",
        name: "OpenAI",
        kind: "openai",
        baseUrl: "https://api.openai.com/v1",
        apiKey: "sk-test",
      },
    ],
    profiles: [
      { id: "prof-1", name: "Fast", connectionId: "conn-1", model: "gpt-5.4-mini" },
      { id: "prof-orphan", name: "Orphan", connectionId: "missing", model: "x" },
    ],
    assignments: { chatProfileId: "prof-1", aiActionsProfileId: "prof-1" },
  };

  it("returns null for null profileId", () => {
    expect(resolveProfile(config, null)).toBeNull();
  });

  it("returns null for a missing profileId", () => {
    expect(resolveProfile(config, "does-not-exist")).toBeNull();
  });

  it("returns null when the profile's connectionId points at no connection", () => {
    expect(resolveProfile(config, "prof-orphan")).toBeNull();
  });

  it("returns the resolved pair for a valid profileId", () => {
    expect(resolveProfile(config, "prof-1")).toEqual({
      connection: config.connections[0],
      model: "gpt-5.4-mini",
    });
  });
});
