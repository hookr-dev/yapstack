import { useEffect, useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { ProfilePicker } from "@/components/ai/ProfilePicker";
import {
  clearChatContextProfile,
  getChatContextProfileId,
  setChatContextProfileId,
} from "@/lib/db";

/**
 * Chat composer's per-conversation Profile picker. Reads the override row
 * from chat_context_settings; writes through setChatContextProfileId.
 * The "Use default" option clears the row so the live Chat Assignment
 * takes over again.
 *
 * Async DB read is local — there's no other consumer of the override and
 * caching it in Zustand would just be a second source of truth. State
 * resets when the contextKey changes.
 */
export function ChatProfilePickerPill({ contextKey }: { contextKey: string }) {
  const aiConfig = useAppStore((s) => s.settings.aiConfig);
  const [override, setOverride] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    if (!contextKey) {
      setOverride(null);
      return;
    }
    getChatContextProfileId(contextKey)
      .then((value) => {
        if (!cancelled) setOverride(value);
      })
      .catch(() => {
        if (!cancelled) setOverride(null);
      });
    return () => {
      cancelled = true;
    };
  }, [contextKey]);

  // Effective value: if an override is set we show that profile; otherwise
  // surface the global Chat assignment so the trigger label is meaningful
  // ("GPT-5 Mini" rather than "Use default") while still letting "Use
  // default" appear in the menu to clear the override.
  const effectiveValue = override ?? aiConfig.assignments.chatProfileId;

  async function handleChange(next: string | null) {
    if (next === null) {
      // "Use default" picked — clear any override and revert to the live
      // global assignment.
      setOverride(null);
      await clearChatContextProfile(contextKey).catch(() => {});
    } else {
      setOverride(next);
      await setChatContextProfileId(contextKey, next).catch(() => {});
    }
  }

  return (
    <ProfilePicker
      profiles={aiConfig.profiles}
      connections={aiConfig.connections}
      value={effectiveValue}
      onChange={handleChange}
      defaultLabel="Use default"
      variant="pill"
      unassignedLabel="No profile"
    />
  );
}
