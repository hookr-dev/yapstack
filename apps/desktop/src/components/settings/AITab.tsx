import { useEffect, useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ConnectionsSubTab } from "@/components/ai/ConnectionsSubTab";
import { ProfilesSubTab } from "@/components/ai/ProfilesSubTab";

type SubTab = "profiles" | "connections";

export function AITab() {
  // Consume the one-shot settingsRequest exactly once on mount. If the user
  // arrived via "Add Connection" from a feature surface, land them on the
  // Connections sub-tab and ask ConnectionsSubTab to open the editor.
  const initialRequest = useAppStore.getState().settingsRequest;
  const setSettingsRequest = useAppStore((s) => s.setSettingsRequest);

  const [tab, setTab] = useState<SubTab>(
    initialRequest === "ai-add-connection" ? "connections" : "profiles",
  );
  const [autoOpenConnectionEditor, setAutoOpenConnectionEditor] = useState(
    initialRequest === "ai-add-connection",
  );

  // Clear the request after consumption so revisiting Settings later doesn't
  // re-trigger the editor.
  useEffect(() => {
    if (initialRequest !== null) setSettingsRequest(null);
    // Intentional: run once on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <Tabs
      value={tab}
      onValueChange={(v) => setTab(v as SubTab)}
      className="flex flex-col gap-4"
    >
      <TabsList variant="line" className="w-full">
        <TabsTrigger value="profiles">Profiles</TabsTrigger>
        <TabsTrigger value="connections">Connections</TabsTrigger>
      </TabsList>

      <TabsContent value="profiles">
        <ProfilesSubTab onGoToConnections={() => setTab("connections")} />
      </TabsContent>

      <TabsContent value="connections">
        <ConnectionsSubTab
          autoOpenEditor={autoOpenConnectionEditor}
          onAutoOpenConsumed={() => setAutoOpenConnectionEditor(false)}
        />
      </TabsContent>
    </Tabs>
  );
}
