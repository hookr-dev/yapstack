import { useAppStore } from "@/stores/appStore";
import { Progress } from "@/components/ui/progress";
import { Loader2 } from "lucide-react";

export function SetupBanner() {
  const enginePhase = useAppStore((s) => s.enginePhase);
  const modelDownloadProgress = useAppStore((s) => s.modelDownloadProgress);
  const selectedModelSize = useAppStore((s) => s.settings.selectedModelSize);

  if (enginePhase !== "downloading" && enginePhase !== "initializing") {
    return null;
  }

  return (
    <div className="mx-4 mt-3 rounded-lg border bg-muted/50 p-3">
      {enginePhase === "downloading" && (
        <>
          <Progress value={modelDownloadProgress ?? 0} className="mb-2 h-2" />
          <p className="text-center text-xs text-muted-foreground">
            Downloading {selectedModelSize} model...
          </p>
        </>
      )}
      {enginePhase === "initializing" && (
        <div className="flex items-center justify-center gap-2">
          <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
          <p className="text-xs text-muted-foreground">
            Loading transcription engine...
          </p>
        </div>
      )}
    </div>
  );
}
