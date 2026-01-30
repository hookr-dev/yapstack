import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateStatus =
  | { available: false }
  | { available: true; version: string; body: string | undefined };

export interface DownloadProgress {
  contentLength: number | undefined;
  downloaded: number;
}

export async function checkForUpdate(): Promise<UpdateStatus> {
  const update = await check();
  if (!update) return { available: false };
  return {
    available: true,
    version: update.version,
    body: update.body ?? undefined,
  };
}

export async function downloadAndInstallUpdate(
  onProgress?: (progress: DownloadProgress) => void,
): Promise<void> {
  const update = await check();
  if (!update) throw new Error("No update available");

  let downloaded = 0;
  let totalLength: number | undefined;
  await update.downloadAndInstall((event) => {
    if (event.event === "Started") {
      downloaded = 0;
      totalLength = event.data.contentLength ?? undefined;
      onProgress?.({ contentLength: totalLength, downloaded: 0 });
    } else if (event.event === "Progress") {
      downloaded += event.data.chunkLength;
      onProgress?.({ contentLength: totalLength, downloaded });
    } else if (event.event === "Finished") {
      onProgress?.({ contentLength: totalLength, downloaded });
    }
  });

  await relaunch();
}
