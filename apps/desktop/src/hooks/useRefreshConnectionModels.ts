import { useState } from "react";
import { toast } from "sonner";
import { useAppStore } from "@/stores/appStore";
import { fetchCustomModels, type Connection } from "@/lib/ai";

export function useRefreshConnectionModels() {
  const updateSettings = useAppStore((s) => s.updateSettings);
  const [refreshingId, setRefreshingId] = useState<string | null>(null);

  async function refresh(connectionId: string): Promise<void> {
    // Read live state — the caller may have just persisted this Connection a tick ago.
    const initial = useAppStore.getState().settings.aiConfig;
    const conn = initial.connections.find((c) => c.id === connectionId);
    if (!conn) return;

    setRefreshingId(connectionId);
    try {
      const models = await fetchCustomModels(conn.baseUrl, conn.apiKey);
      // Re-resolve target — another window may have mutated state during the fetch.
      const latest = useAppStore.getState().settings.aiConfig;
      const target = latest.connections.find((c) => c.id === connectionId);
      if (!target) return;
      const next: Connection = {
        id: target.id,
        name: target.name,
        kind: target.kind,
        baseUrl: target.baseUrl,
        apiKey: target.apiKey,
        availableModels: models,
        fetchedAt: new Date().toISOString(),
      };
      const nextConnections = latest.connections.map((c) =>
        c.id === connectionId ? next : c,
      );
      updateSettings({
        aiConfig: { ...latest, connections: nextConnections },
      });
      if (models.length === 0) {
        toast.warning(
          `${target.name}: server reported no models. You can still type a model name manually in a Profile.`,
        );
      } else {
        toast.success(
          `${target.name}: cached ${models.length} model${models.length === 1 ? "" : "s"}.`,
        );
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      const latest = useAppStore.getState().settings.aiConfig;
      const target = latest.connections.find((c) => c.id === connectionId);
      if (target) {
        const next: Connection = { ...target, fetchError: message };
        const nextConnections = latest.connections.map((c) =>
          c.id === connectionId ? next : c,
        );
        updateSettings({
          aiConfig: { ...latest, connections: nextConnections },
        });
      }
      toast.error(`Couldn't fetch models: ${message}`);
    } finally {
      setRefreshingId(null);
    }
  }

  return { refresh, refreshingId };
}
