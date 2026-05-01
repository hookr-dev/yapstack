import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";

/**
 * Reveal an audio file (or any file path) in the OS file manager. On macOS
 * this opens Finder with the file selected; on Windows/Linux it does the
 * platform-equivalent. Failures are logged and toasted — callers don't need
 * their own try/catch.
 *
 * Used by both the session header and the dictation history list so the two
 * surfaces stay behaviorally identical.
 */
export async function revealAudioFile(path: string): Promise<void> {
  try {
    await revealItemInDir(path);
  } catch (e) {
    console.error("Failed to reveal file:", e);
    toast.error("Couldn't reveal file");
  }
}
