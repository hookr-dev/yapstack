import { useEffect, useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Progress } from "@/components/ui/progress";
import { Download, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { ParakeetVariantDto } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";

/// Parakeet model picker. Mirrors the Whisper [`ModelSection`] layout so the
/// switcher feels the same regardless of which engine is active. The active
/// engine is tracked separately via `selectedEngine`; clicking a Parakeet
/// model row switches both the variant and the engine.
export function ParakeetModelSection() {
  const selectedEngine = useAppStore((s) => s.settings.selectedEngine);
  const selectedVariant = useAppStore(
    (s) => s.settings.selectedParakeetVariant,
  );
  const parakeetModels = useAppStore((s) => s.parakeetModels);
  const modelDownloadProgress = useAppStore((s) => s.modelDownloadProgress);
  const enginePhase = useAppStore((s) => s.enginePhase);
  const downloadParakeet = useAppStore((s) => s.downloadParakeetModel);
  const deleteParakeet = useAppStore((s) => s.deleteParakeetModel);
  const switchEngine = useAppStore((s) => s.switchEngine);

  const isDownloading = modelDownloadProgress !== null;
  const [downloadingVariant, setDownloadingVariant] =
    useState<ParakeetVariantDto | null>(null);

  useEffect(() => {
    if (!isDownloading && downloadingVariant !== null)
      setDownloadingVariant(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isDownloading]);

  const handleSwitch = async (variant: ParakeetVariantDto) => {
    try {
      // Variant change + engine activation in one go.
      useAppStore
        .getState()
        .updateSettings({ selectedParakeetVariant: variant });
      if (selectedEngine !== "Parakeet") {
        await switchEngine("Parakeet");
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDownload = async (variant: ParakeetVariantDto) => {
    setDownloadingVariant(variant);
    try {
      await downloadParakeet(variant);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDelete = async (variant: ParakeetVariantDto) => {
    if (
      variant === selectedVariant &&
      selectedEngine === "Parakeet" &&
      enginePhase === "ready"
    ) {
      toast.error("Cannot delete the active model");
      return;
    }
    try {
      await deleteParakeet(variant);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="space-y-1">
      {parakeetModels.map((model) => {
        const isActive =
          model.variant === selectedVariant &&
          selectedEngine === "Parakeet" &&
          model.downloaded &&
          enginePhase === "ready";
        const isClickable =
          !isDownloading &&
          !isActive &&
          (model.variant !== selectedVariant ||
            selectedEngine !== "Parakeet");

        const handleRowClick = () => {
          if (!isClickable) return;
          if (model.downloaded) {
            handleSwitch(model.variant);
          } else {
            handleDownload(model.variant);
          }
        };

        return (
          <div
            key={model.variant}
            className={`flex items-center justify-between rounded-md px-2 py-1.5 ${
              isClickable
                ? "cursor-pointer hover:bg-muted/50"
                : "hover:bg-muted/50"
            }`}
            onClick={handleRowClick}
          >
            <div className="flex items-center gap-2">
              <span
                className={
                  isActive
                    ? "text-sm font-medium text-foreground"
                    : "text-sm text-muted-foreground"
                }
              >
                {model.display_name}
              </span>
              <Badge variant="secondary" className="text-xs">
                {formatBytes(model.approximate_size_bytes)}
              </Badge>
            </div>

            <div className="flex items-center gap-1">
              {isDownloading &&
              (downloadingVariant === model.variant ||
                (downloadingVariant === null &&
                  model.variant === selectedVariant)) ? (
                <div className="w-24">
                  <Progress
                    value={modelDownloadProgress ?? 0}
                    className="h-2"
                  />
                </div>
              ) : isActive ? (
                <Badge
                  variant="outline"
                  className="border-green-600 text-xs text-green-600"
                >
                  Active
                </Badge>
              ) : model.downloaded ? (
                <Button
                  variant="ghost"
                  size="icon-xs"
                  disabled={isDownloading}
                  onClick={(e) => {
                    e.stopPropagation();
                    handleDelete(model.variant);
                  }}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              ) : (
                <Button
                  variant="ghost"
                  size="icon-xs"
                  disabled={isDownloading}
                  onClick={(e) => {
                    e.stopPropagation();
                    handleDownload(model.variant);
                  }}
                >
                  <Download className="h-3.5 w-3.5" />
                </Button>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
