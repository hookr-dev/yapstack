import { useEffect } from "react";
import { checkForUpdate } from "@/lib/updater";
import { useAppStore } from "@/stores/appStore";
import { trackUpdateAvailable } from "@/lib/analytics";

const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000; // 4 hours

async function doCheck() {
  try {
    const status = await checkForUpdate();
    if (status.available) {
      trackUpdateAvailable({ version: status.version });
      useAppStore.getState().setUpdateAvailable({
        version: status.version,
        body: status.body,
      });
    }
  } catch {
    // Silently ignore — offline, rate-limited, no published release, etc.
  }
}

export function useUpdateCheck() {
  useEffect(() => {
    const timer = setTimeout(doCheck, 5000);
    const interval = setInterval(doCheck, CHECK_INTERVAL_MS);

    return () => {
      clearTimeout(timer);
      clearInterval(interval);
    };
  }, []);
}
