import { useEffect } from "react";
import { useAppStore } from "@/stores/appStore";
import { SHORTCUTS, SHORTCUT_MAP, getBinding, eventToBinding, shortcutCaptureActive } from "@/lib/shortcuts";
import { trackShortcutUsed } from "@/lib/analytics";

/** Returns true if focus is in an input, textarea, or contenteditable element. */
function isInputFocused(): boolean {
  const el = document.activeElement;
  if (!el) return false;
  const tag = el.tagName.toLowerCase();
  if (tag === "input" || tag === "textarea") return true;
  if ((el as HTMLElement).isContentEditable) return true;
  return false;
}

/**
 * Build a reverse map from binding string → shortcut ID.
 * Only includes in-app (non-global, non-displayOnly) shortcuts.
 */
function buildBindingMap(overrides: Record<string, string>): Map<string, string> {
  const map = new Map<string, string>();
  for (const shortcut of SHORTCUTS) {
    if (shortcut.isGlobal) continue;
    const binding = getBinding(shortcut.id, overrides);
    if (binding) map.set(binding, shortcut.id);
  }
  return map;
}

/** Handles in-app keyboard shortcuts via capture-phase keydown listener. Mounted in AppLayout. */
export function useKeyboardShortcuts() {
  const overrides = useAppStore((s) => s.settings.shortcutBindings);

  useEffect(() => {
    const bindingMap = buildBindingMap(overrides);

    function handler(e: KeyboardEvent) {
      // Skip when shortcut capture is active (rebinding in settings)
      if (shortcutCaptureActive.current) return;

      const binding = eventToBinding(e);
      if (!binding) return;

      const id = bindingMap.get(binding);
      if (!id) return;

      if (isInputFocused() && !SHORTCUT_MAP.get(id)?.allowInEditor) return;

      trackShortcutUsed({ shortcut_id: id });
      const s = useAppStore.getState();

      // stopPropagation also prevents TipTap's keymap from firing on the same key.
      const consume = () => {
        e.preventDefault();
        e.stopPropagation();
      };

      switch (id) {
        case "command-palette":
          consume();
          window.dispatchEvent(new CustomEvent("yapstack:toggle-search"));
          break;

        case "toggle-sidebar":
          consume();
          s.toggleSidebar();
          break;

        case "open-settings":
          consume();
          s.navigateTo("settings");
          break;

        case "go-back":
          // Only go back from detail or settings
          if (s.currentView === "note-detail" || s.currentView === "settings") {
            consume();
            s.navigateTo("note-list");
          }
          break;

        case "filter-all":
          consume();
          s.setListFilter({ type: "all" });
          s.navigateTo("note-list");
          break;

        case "filter-pinned":
          consume();
          s.setListFilter({ type: "pinned" });
          s.navigateTo("note-list");
          break;

        case "new-note":
          consume();
          s.createManualNote();
          break;

        case "stop-recording":
          consume();
          s.stopActiveSession();
          break;

        case "toggle-chat":
          consume();
          window.dispatchEvent(new CustomEvent("yapstack:toggle-chat"));
          break;

        case "pin-session":
          consume();
          if (s.currentView === "note-detail" && s.selectedSessionId) {
            s.togglePin(s.selectedSessionId);
          }
          break;

        case "delete-session":
          consume();
          if (s.currentView === "note-detail" && s.selectedSessionId) {
            window.dispatchEvent(new CustomEvent("yapstack:confirm-delete-session"));
          }
          break;
      }
    }

    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, [overrides]);
}
