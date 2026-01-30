import { useEffect, useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Progress } from "@/components/ui/progress";
import { Download, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { ModelSizeDto } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";

export function ModelSection() {
  const selectedModelSize = useAppStore((s) => s.settings.selectedModelSize);
  const models = useAppStore((s) => s.models);
  const modelDownloadProgress = useAppStore((s) => s.modelDownloadProgress);
  const enginePhase = useAppStore((s) => s.enginePhase);
  const deleteModel = useAppStore((s) => s.deleteModel);
  const switchModel = useAppStore((s) => s.switchModel);
  const downloadModel = useAppStore((s) => s.downloadModel);

  const isDownloading = modelDownloadProgress !== null;
  const [downloadingSize, setDownloadingSize] = useState<ModelSizeDto | null>(null);

  // Clear state when download finishes
  useEffect(() => {
    if (!isDownloading && downloadingSize !== null) setDownloadingSize(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only react to download completion, not size changes
  }, [isDownloading]);

  const handleSwitchModel = async (size: ModelSizeDto) => {
    try {
      await switchModel(size);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDeleteModel = async (size: ModelSizeDto) => {
    if (size === selectedModelSize && enginePhase === "ready") {
      toast.error("Cannot delete the active model");
      return;
    }
    try {
      await deleteModel(size);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDownloadModel = async (size: ModelSizeDto) => {
    setDownloadingSize(size);
    try {
      await downloadModel(size);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="space-y-1">
      {models.map((model) => {
        const isActive =
          model.size === selectedModelSize &&
          model.downloaded &&
          enginePhase === "ready";
        const isClickable =
          !isDownloading &&
          !isActive &&
          model.size !== selectedModelSize;

        const handleRowClick = () => {
          if (!isClickable) return;
          if (model.downloaded) {
            handleSwitchModel(model.size);
          } else {
            handleDownloadModel(model.size);
          }
        };

        return (
          <div
            key={model.size}
            className={`flex items-center justify-between rounded-md px-2 py-1.5 ${isClickable
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
              {model.size === "Small" && (
                <Badge className="bg-primary/15 text-[10px] text-primary border-primary/20">
                  Recommended
                </Badge>
              )}
            </div>

            <div className="flex items-center gap-1">
              {isDownloading && downloadingSize === model.size ? (
                <div className="w-24">
                  <Progress value={modelDownloadProgress ?? 0} className="h-2" />
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
                    handleDeleteModel(model.size);
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
                  handleDownloadModel(model.size);
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
