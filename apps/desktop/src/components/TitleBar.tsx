import { useState, useEffect, useCallback } from "react";
import { useAppStore } from "@/stores/appStore";
import { useCreateSession } from "@/hooks/useCreateSession";
import { Progress } from "@/components/ui/progress";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";
import { formatBytes, SOURCE_LABELS_FULL, formatShortcutDisplay, isMac } from "@/lib/utils";
import { getBinding } from "@/lib/shortcuts";
import { Search, PanelLeftClose, PanelLeftOpen, Plus, PenLine, Minus, Square, X } from "lucide-react";
import { SearchCommand } from "@/components/SearchCommand";
import { BackfillDropdown } from "@/components/BackfillDropdown";
import { getCurrentWindow } from "@tauri-apps/api/window";

function StatusDot({ color }: { color: string }) {
  return (
    <span
      className={`inline-block h-2 w-2 rounded-full ${color}`}
      aria-hidden
    />
  );
}

function BufferDetail({
  label,
  info,
}: {
  label: string;
  info: {
    available_seconds: number;
    capacity_seconds: number;
    capacity_samples: number;
  } | null;
}) {
  if (!info) {
    return (
      <div className="space-y-1">
        <span className="text-xs text-muted-foreground">{label}</span>
        <p className="text-xs">N/A</p>
      </div>
    );
  }
  const pct =
    info.capacity_seconds > 0
      ? (info.available_seconds / info.capacity_seconds) * 100
      : 0;
  const memBytes = info.capacity_samples * 4;
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between">
        <span className="text-xs text-muted-foreground">{label}</span>
        <span className="font-mono text-xs">
          {Math.floor(info.available_seconds)}s / {Math.floor(info.capacity_seconds)}s
        </span>
      </div>
      <Progress value={pct} className="h-1.5" />
      <p className="text-[10px] text-muted-foreground/60">{formatBytes(memBytes)}</p>
    </div>
  );
}

function BufferPopoverContent() {
  const bufferInfo = useAppStore((s) => s.bufferInfo);
  const captureStatus = useAppStore((s) => s.captureStatus);
  const captureSource = useAppStore((s) => s.settings.captureSource);
  const selectedMicDeviceId = useAppStore((s) => s.settings.selectedMicDeviceId);
  const devices = useAppStore((s) => s.devices);
  const sourceLabel = SOURCE_LABELS_FULL[captureSource];

  return (
    <div className="space-y-3">
      <BufferDetail label="Mic Capture Buffer" info={bufferInfo?.mic ?? null} />
      <BufferDetail label="System Capture Buffer" info={bufferInfo?.system ?? null} />
      <div className="space-y-1 border-t pt-2">
        <div className="flex items-center justify-between">
          <span className="text-xs text-muted-foreground">Source</span>
          <span className="text-xs">{sourceLabel}</span>
        </div>
        {captureStatus?.mic_active && (
          <div className="flex items-center justify-between">
            <span className="text-xs text-muted-foreground">Device</span>
            <span className="max-w-[140px] truncate text-xs">
              {(selectedMicDeviceId
                ? (devices.find((d) => d.id === selectedMicDeviceId)
                  ?? devices.find((d) => d.name === selectedMicDeviceId))?.name
                : null) ?? "Default"}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}

export function TitleBar() {
  const enginePhase = useAppStore((s) => s.enginePhase);
  const engineError = useAppStore((s) => s.engineError);
  const captureStatus = useAppStore((s) => s.captureStatus);
  const modelDownloadProgress = useAppStore((s) => s.modelDownloadProgress);

  const sidebarCollapsed = useAppStore((s) => s.settings.sidebarCollapsed);
  const toggleSidebar = useAppStore((s) => s.toggleSidebar);

  const createManualNote = useAppStore((s) => s.createManualNote);

  const shortcutBindings = useAppStore((s) => s.settings.shortcutBindings);
  const availableSeconds = useAppStore((s) =>
    Math.floor(Math.max(
      s.bufferInfo?.mic?.available_seconds ?? 0,
      s.bufferInfo?.system?.available_seconds ?? 0,
    )),
  );
  const [searchOpen, setSearchOpen] = useState(false);
  const { canCreate, handleNew } = useCreateSession();

  const onCreateSession = useCallback(() => handleNew(), [handleNew]);

  useEffect(() => {
    window.addEventListener("yapstack:create-session", onCreateSession);
    return () => window.removeEventListener("yapstack:create-session", onCreateSession);
  }, [onCreateSession]);

  useEffect(() => {
    const handler = () => setSearchOpen(true);
    window.addEventListener("yapstack:toggle-search", handler);
    return () => window.removeEventListener("yapstack:toggle-search", handler);
  }, []);

  let dotColor = "bg-muted-foreground";
  let statusText = "Idle";

  if (enginePhase === "downloading") {
    dotColor = "bg-yellow-500";
    const pct = modelDownloadProgress ?? 0;
    statusText = `Downloading model (${Math.round(pct)}%)`;
  } else if (enginePhase === "initializing") {
    dotColor = "bg-yellow-500";
    statusText = "Loading engine...";
  } else if (enginePhase === "error") {
    dotColor = "bg-red-500";
    statusText = engineError ?? "Error";
  } else if (enginePhase === "ready") {
    if (captureStatus?.state === "Capturing") {
      dotColor = "bg-green-500";
      statusText = "Listening";
    } else if (captureStatus?.state === "Error") {
      dotColor = "bg-red-500";
      statusText = captureStatus.error_message ?? "Capture error";
    } else {
      dotColor = "bg-yellow-500";
      statusText = "Engine ready";
    }
  } else {
    dotColor = "bg-muted-foreground";
    statusText = "Setting up...";
  }

  return (
    <>
      <div className="flex h-[38px] shrink-0 items-center border-b px-2" data-tauri-drag-region>
        {/* Traffic light padding (macOS only) */}
        {isMac && <div className="w-[78px] shrink-0" data-tauri-drag-region />}

        {/* Sidebar toggle */}
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={toggleSidebar}
              className="shrink-0 text-muted-foreground"
            >
              {sidebarCollapsed ? (
                <PanelLeftOpen className="h-3.5 w-3.5" />
              ) : (
                <PanelLeftClose className="h-3.5 w-3.5" />
              )}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{sidebarCollapsed ? "Show sidebar" : "Hide sidebar"} ({formatShortcutDisplay(getBinding("toggle-sidebar", shortcutBindings))})</TooltipContent>
        </Tooltip>

        {sidebarCollapsed && (
          <>
            <div className="mx-1.5 h-4 w-px bg-border" />
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  disabled={!canCreate}
                  onClick={() => handleNew()}
                  className="text-muted-foreground"
                >
                  <Plus className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>New session</TooltipContent>
            </Tooltip>
            <BackfillDropdown
              availableSeconds={availableSeconds}
              canCreate={canCreate}
              onBackfill={handleNew}
            />
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  onClick={() => createManualNote()}
                  className="text-muted-foreground"
                >
                  <PenLine className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>New note ({formatShortcutDisplay(getBinding("new-note", shortcutBindings))})</TooltipContent>
            </Tooltip>
          </>
        )}

        {/* Spacer */}
        <div className="flex-1" data-tauri-drag-region />

        <div className="flex items-center gap-1">
            {/* Status dot + popover */}
            <Popover>
              <PopoverTrigger asChild>
                <button className="flex items-center gap-1.5 rounded px-2 py-0.5 text-xs text-muted-foreground hover:bg-muted">
                  <StatusDot color={dotColor} />
                  <span className="max-w-[180px] truncate">{statusText}</span>
                </button>
              </PopoverTrigger>
              <PopoverContent className="w-64 p-3" align="end">
                <BufferPopoverContent />
              </PopoverContent>
            </Popover>

            {/* Search button */}
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  onClick={() => setSearchOpen(true)}
                >
                  <Search className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Search ({formatShortcutDisplay(getBinding("command-palette", shortcutBindings))})</TooltipContent>
            </Tooltip>
          </div>

          {/* Windows window controls (no native decorations) */}
          {!isMac && (
            <div className="ml-1 flex items-center">
              <button
                onClick={() => getCurrentWindow().minimize()}
                className="inline-flex h-[38px] w-[46px] items-center justify-center text-muted-foreground hover:bg-muted"
              >
                <Minus className="h-3.5 w-3.5" />
              </button>
              <button
                onClick={() => getCurrentWindow().toggleMaximize()}
                className="inline-flex h-[38px] w-[46px] items-center justify-center text-muted-foreground hover:bg-muted"
              >
                <Square className="h-3 w-3" />
              </button>
              <button
                onClick={() => getCurrentWindow().close()}
                className="inline-flex h-[38px] w-[46px] items-center justify-center text-muted-foreground hover:bg-red-500 hover:text-white"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          )}
      </div>

      <SearchCommand open={searchOpen} onOpenChange={setSearchOpen} />
    </>
  );
}
