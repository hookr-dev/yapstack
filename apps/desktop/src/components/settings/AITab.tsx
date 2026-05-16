import { useState } from "react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ConnectionsSubTab } from "@/components/ai/ConnectionsSubTab";
import { ProfilesSubTab } from "@/components/ai/ProfilesSubTab";

type SubTab = "profiles" | "connections";

export function AITab() {
  const [tab, setTab] = useState<SubTab>("profiles");

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
        <ConnectionsSubTab />
      </TabsContent>
    </Tabs>
  );
}
