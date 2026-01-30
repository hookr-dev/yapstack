import { useAppStore } from "@/stores/appStore";
import { toast } from "sonner";

/** Provides `canCreate` flag and `handleNew` function for starting recording sessions. */
export function useCreateSession() {
  const enginePhase = useAppStore((s) => s.enginePhase);
  const captureStatus = useAppStore((s) => s.captureStatus);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const createAndStartSession = useAppStore((s) => s.createAndStartSession);

  const isReady =
    enginePhase === "ready" && captureStatus?.state === "Capturing";
  const canCreate = isReady && !activeSessionId;

  const handleNew = async (backfillSeconds?: number) => {
    try {
      await createAndStartSession(backfillSeconds, "sidebar");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  return { canCreate, handleNew };
}
