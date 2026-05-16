import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ConnectionsSubTab } from "@/components/ai/ConnectionsSubTab";

export function AITab() {
  return (
    <Tabs defaultValue="profiles" className="flex flex-col gap-4">
      <TabsList variant="line" className="w-full">
        <TabsTrigger value="profiles">Profiles</TabsTrigger>
        <TabsTrigger value="connections">Connections</TabsTrigger>
      </TabsList>

      <TabsContent value="profiles">
        <ProfilesPlaceholder />
      </TabsContent>

      <TabsContent value="connections">
        <ConnectionsSubTab />
      </TabsContent>
    </Tabs>
  );
}

function ProfilesPlaceholder() {
  return (
    <div className="rounded-md border border-dashed border-border bg-card px-6 py-8 text-center">
      <h3 className="text-sm font-medium">Profiles</h3>
      <p className="mx-auto mt-1 max-w-xs text-xs text-muted-foreground leading-relaxed">
        Profile management lands in the next commit. For now, create
        Connections in the Connections tab.
      </p>
    </div>
  );
}
