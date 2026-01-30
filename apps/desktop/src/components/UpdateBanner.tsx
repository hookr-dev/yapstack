import { useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { downloadAndInstallUpdate } from "@/lib/updater";
import { trackUpdateInstallStarted, trackUpdateInstallFailed } from "@/lib/analytics";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Download, X, Loader2 } from "lucide-react";

export function UpdateBanner() {
  const updateAvailable = useAppStore((s) => s.updateAvailable);
  const updateDismissedVersion = useAppStore((s) => s.updateDismissedVersion);
  const dismissUpdate = useAppStore((s) => s.dismissUpdate);
  const isRecording = useAppStore((s) => s.liveTranscriptionActive);

  const [installing, setInstalling] = useState(false);
  const [showRecordingWarning, setShowRecordingWarning] = useState(false);

  if (!updateAvailable || updateDismissedVersion === updateAvailable.version) {
    return null;
  }

  const doInstall = async () => {
    setInstalling(true);
    trackUpdateInstallStarted({ version: updateAvailable.version });
    try {
      await downloadAndInstallUpdate();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      trackUpdateInstallFailed({ version: updateAvailable.version, error: msg });
      setInstalling(false);
    }
  };

  const handleClick = () => {
    if (installing) return;
    if (isRecording) {
      setShowRecordingWarning(true);
      return;
    }
    doInstall();
  };

  return (
    <>
      <div
        role="button"
        tabIndex={0}
        className="group relative mb-1 flex cursor-pointer items-center gap-2 rounded-md bg-emerald-500/10 px-2 py-1.5 text-xs text-emerald-400 transition-colors hover:bg-emerald-500/20"
        onClick={handleClick}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") handleClick(); }}
      >
        {installing ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
        ) : (
          <Download className="h-3.5 w-3.5 shrink-0" />
        )}
        <span className="min-w-0 truncate">
          {installing ? "Installing..." : `v${updateAvailable.version} available`}
        </span>
        {!installing && (
          <button
            className="ml-auto shrink-0 rounded p-0.5 opacity-0 transition-opacity hover:bg-emerald-500/20 group-hover:opacity-100"
            onClick={(e) => {
              e.stopPropagation();
              dismissUpdate();
            }}
            aria-label="Dismiss update"
          >
            <X className="h-3 w-3" />
          </button>
        )}
      </div>

      <AlertDialog open={showRecordingWarning} onOpenChange={setShowRecordingWarning}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Recording in progress</AlertDialogTitle>
            <AlertDialogDescription>
              A recording session is currently active. Installing the update will
              restart the app and stop the recording. Do you want to continue?
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setShowRecordingWarning(false);
                doInstall();
              }}
            >
              Install Anyway
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
