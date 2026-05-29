import { useEffect, useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { Separator } from "@/components/ui/separator";
import { AssignmentsSection } from "@/components/ai/AssignmentsSection";
import { ConnectionsSection } from "@/components/ai/ConnectionsSection";
import { ProfilesSection } from "@/components/ai/ProfilesSection";

export function AITab() {
  // Consume the one-shot settingsRequest exactly once on mount. If the user
  // arrived via "Add Connection" from a feature surface, open the Connection
  // editor automatically.
  const initialRequest = useAppStore.getState().settingsRequest;
  const setSettingsRequest = useAppStore((s) => s.setSettingsRequest);

  const [autoOpenConnectionEditor, setAutoOpenConnectionEditor] = useState(
    initialRequest === "ai-add-connection",
  );

  useEffect(() => {
    if (initialRequest !== null) setSettingsRequest(null);
    // Intentional: run once on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <>
      <ConnectionsSection
        autoOpenEditor={autoOpenConnectionEditor}
        onAutoOpenConsumed={() => setAutoOpenConnectionEditor(false)}
      />
      <Separator />
      <ProfilesSection />
      <Separator />
      <AssignmentsSection />
    </>
  );
}
