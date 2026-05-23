import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { X, AlertTriangle, Loader2 } from "lucide-react";
import {
  EVENTS,
  WINDOWS,
  emitEvent,
  listenEvent,
  type InsightStateEvent,
} from "@/lib/events";
import { useOverlayStyles } from "@/hooks/useOverlayStyles";
import { markdownToBasicHtml } from "@/lib/ai";
import { commands } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { log } from "@/lib/logger";
import { InsightSwitcher } from "@/components/InsightSwitcher";

const EMPTY: InsightStateEvent = {
  insightName: "",
  status: "idle",
  content: null,
  generatedAt: null,
  error: null,
  currentInsightId: null,
  slots: [],
};

/** Height (logical px) of the interactive header strip. The rest of the
 *  overlay is click-through. Keep in sync with the `<header>` rendered size
 *  below — `h-[30px]` plus border. */
const HEADER_HEIGHT_LOGICAL = 30;

/** How often we poll the OS cursor position to update click-through. 60 ms
 *  is ~17 Hz — well below human-noticeable lag, well within Tauri IPC
 *  budget. */
const CURSOR_POLL_MS = 60;

/** Approx. rendered height (logical px) of one switcher dropdown row, plus the
 *  popover's own padding/offset. Used to grow the overlay window so the
 *  dropdown isn't clipped by the small window's bounds (a webview can't paint
 *  outside its OS window). */
const DROPDOWN_ROW_HEIGHT = 30;
const DROPDOWN_CHROME = HEADER_HEIGHT_LOGICAL + 20;
/** Cap how many rows we size the window to before the popover starts to
 *  scroll internally — keeps the overlay from ballooning with many Insights. */
const DROPDOWN_MAX_ROWS = 8;

/** Auto-fit bounds (logical px). The window height tracks the rendered card so
 *  the body never scrolls. MIN keeps a one-line result from collapsing; MAX is
 *  a safety ceiling so a runaway response can't grow the window to fill the
 *  screen — beyond it the body falls back to scrolling (rare: the engine prompt
 *  already caps output at ~12 lines). Keep MIN at or above the window's
 *  configured `minHeight` so `setSize` isn't clamped out from under us. */
const MIN_FIT_HEIGHT = 44;
const MAX_FIT_HEIGHT = 560;

export function InsightOverlay() {
  useOverlayStyles();
  const [state, setState] = useState<InsightStateEvent>(EMPTY);

  // Receive state from main window.
  useEffect(() => {
    const unlisten = listenEvent(EVENTS.INSIGHT_STATE, setState);
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  // ---- Auto-fit the window height to the rendered content ----
  // The body is click-through, so the user can't scroll it. Instead of
  // estimating height from font metrics (fragile: Markdown wrapping, list and
  // paragraph margins, and per-element line-heights don't reduce to a simple
  // line count), we measure the actual laid-out card with getBoundingClientRect
  // and mirror the OS window height to it. The measurement already accounts for
  // every padding, border, margin and wrap, so spacing is preserved exactly.
  const rootRef = useRef<HTMLDivElement>(null);
  const lastAppliedHeightRef = useRef(0);
  // While the switcher popover is open the card temporarily fills the window
  // (see the JSX `h-screen` toggle) so the dropdown has room; pause auto-fit so
  // it doesn't fight the dropdown-grow. The state drives the className; the ref
  // mirror is read synchronously by the fit loop and the click-through poll.
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const switcherOpenRef = useRef(false);

  // Set the window height (logical px), clamped to the fit bounds. Width is
  // preserved (auto-fit only touches height; the window stays user-resizable
  // horizontally). The diff guard makes repeated observer fires cheap.
  const applyHeight = useCallback(async (rawHeight: number) => {
    const h = Math.min(
      Math.max(Math.ceil(rawHeight), MIN_FIT_HEIGHT),
      MAX_FIT_HEIGHT,
    );
    if (Math.abs(h - lastAppliedHeightRef.current) < 1) return;
    lastAppliedHeightRef.current = h;
    try {
      const win = getCurrentWindow();
      const [size, scale] = await Promise.all([
        win.outerSize(),
        win.scaleFactor(),
      ]);
      const logicalW = Math.round(size.width / scale);
      await win.setSize(new LogicalSize(logicalW, h));
    } catch (e) {
      log.warn(
        `overlay: auto-fit setSize failed — ${e instanceof Error ? e.message : String(e)}`,
        "insights",
      );
    }
  }, []);

  const fitToContent = useCallback(() => {
    // The dropdown owns sizing while open; don't shrink the window under it.
    if (switcherOpenRef.current) return;
    const root = rootRef.current;
    if (!root) return;
    void applyHeight(root.getBoundingClientRect().height);
  }, [applyHeight]);

  useEffect(() => {
    const root = rootRef.current;
    if (!root || typeof ResizeObserver === "undefined") return;
    fitToContent(); // initial fit (e.g. the "Waiting…" placeholder)
    const ro = new ResizeObserver(() => fitToContent());
    ro.observe(root);
    return () => ro.disconnect();
  }, [fitToContent]);

  // Region click-through: the header strip stays interactive; everything
  // below it passes mouse events through to whatever window is under us.
  // Tauri's JS `setIgnoreCursorEvents` routes through the pre-conversion
  // NSWindow handle and silently no-ops on `tauri-nspanel` panels, so we
  // call a custom Rust command that goes through the panel handle.
  const lastClickThroughRef = useRef<boolean | null>(null);
  // `switcherOpenRef` (declared above) is true while the InsightSwitcher popover
  // is open. While true the polling tick early-returns so a body-region cursor
  // doesn't flip click-through ON underneath the popover items; the popover's
  // onOpenChange also calls `applyClickThroughRef` to force OFF immediately.
  // Imperative handle the JSX layer can use to force a click-through state
  // outside the polling loop. Assigned by the effect below; called by the
  // switcher's onOpenChange when the dropdown opens.
  const applyClickThroughRef = useRef<((passThrough: boolean) => Promise<void>) | null>(null);
  useEffect(() => {
    const win = getCurrentWindow();
    let cancelled = false;

    async function applyClickThrough(passThrough: boolean) {
      if (lastClickThroughRef.current === passThrough) return;
      lastClickThroughRef.current = passThrough;
      log.debug(
        `overlay: click-through ${passThrough ? "ON (body)" : "OFF (header/popover)"}`,
        "insights",
      );
      try {
        const result = await commands.setOverlayIgnoreCursorEvents(
          WINDOWS.INSIGHT,
          passThrough,
        );
        if (result.status !== "ok") {
          log.warn(
            `overlay: setOverlayIgnoreCursorEvents → ${JSON.stringify(result.error)}`,
            "insights",
          );
        }
      } catch (e) {
        log.warn(
          `overlay: setOverlayIgnoreCursorEvents threw — ${e instanceof Error ? e.message : String(e)}`,
          "insights",
        );
      }
    }
    applyClickThroughRef.current = applyClickThrough;

    let tickCount = 0;
    async function tick() {
      if (cancelled) return;
      // Polling is paused while the popover is open. The popover owns the
      // click-through state during its lifetime; the polling tick can't
      // accidentally toggle it back ON if the cursor moves into the body.
      if (switcherOpenRef.current) return;
      try {
        const cursor = await commands.getCursorPosition();
        if (cursor.status !== "ok") {
          if (tickCount === 0) {
            log.warn(
              `overlay: getCursorPosition first call returned error — ${JSON.stringify(cursor.error)}`,
              "insights",
            );
          }
          tickCount++;
          return;
        }
        const [cx, cy] = cursor.data;
        const [pos, size, scale] = await Promise.all([
          win.outerPosition(),
          win.outerSize(),
          win.scaleFactor(),
        ]);
        const headerH = HEADER_HEIGHT_LOGICAL * scale;
        const inHeader =
          cx >= pos.x &&
          cx <= pos.x + size.width &&
          cy >= pos.y &&
          cy <= pos.y + headerH;
        // passThrough = !inHeader. Header → interactive, body → click-through.
        await applyClickThrough(!inHeader);
        tickCount++;
      } catch (e) {
        if (tickCount === 0) {
          log.warn(
            `overlay: cursor poll first tick threw — ${e instanceof Error ? e.message : String(e)}`,
            "insights",
          );
        }
        tickCount++;
      }
    }

    log.info("overlay: cursor-poll starting", "insights");
    // Start fully click-through — cursor is almost certainly NOT in the
    // header on first show, so this is the safe default until the first
    // poll lands.
    void applyClickThrough(true);
    const intervalId = window.setInterval(tick, CURSOR_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
      log.debug("overlay: cursor-poll stopped", "insights");
    };
  }, []);

  function handleClose() {
    void emitEvent(EVENTS.INSIGHT_HIDE_REQUEST);
  }

  function handleSwitch(next: string | null) {
    void emitEvent(EVENTS.INSIGHT_CHANGE_ACTIVE, { insightId: next });
  }

  // The switcher dropdown is portalled into this (small) overlay window, so a
  // list taller than the auto-fit height would be clipped by the OS window
  // bounds. Grow the window to fit the dropdown when it opens (keeping whatever
  // the content already needs, if taller). On close, auto-fit shrinks it back.
  function growForDropdown(rowCount: number) {
    const dropdownH =
      DROPDOWN_CHROME + Math.min(rowCount, DROPDOWN_MAX_ROWS) * DROPDOWN_ROW_HEIGHT;
    void applyHeight(Math.max(lastAppliedHeightRef.current, dropdownH));
  }

  return (
    <div
      ref={rootRef}
      // Content-sized: the window height tracks this card (auto-fit above), so
      // `max-height` is the only ceiling and the body doesn't scroll. While the
      // switcher is open we let the card fill the (grown) window via `h-screen`
      // so the dropdown floats over the card surface, not transparent space.
      style={switcherOpen ? undefined : { maxHeight: `${MAX_FIT_HEIGHT}px` }}
      className={cn(
        "w-screen overflow-hidden rounded-lg border border-border/60 bg-card/75 text-card-foreground shadow-xl backdrop-blur-lg flex flex-col",
        switcherOpen && "h-screen",
      )}
    >
      {/* Header — drag region + status + close. Sized to match
          HEADER_HEIGHT_LOGICAL so the click-through hit test is accurate.
          A subtle inner darker tint lifts the header above the body. */}
      <header
        data-tauri-drag-region
        className="flex h-[30px] shrink-0 items-center justify-between gap-2 border-b border-border/60 bg-black/15 px-2 select-none"
      >
        <div className="flex min-w-0 items-center">
          <InsightSwitcher
            currentName={state.insightName}
            slots={state.slots}
            currentInsightId={state.currentInsightId}
            onChange={handleSwitch}
            onOpenChange={(open) => {
              switcherOpenRef.current = open;
              setSwitcherOpen(open);
              if (open) {
                // Force click-through OFF the instant the popover opens — don't
                // wait up to 60 ms for the next polling tick. The ref-imperative
                // call also primes lastClickThroughRef so resumed polling (after
                // close) won't re-fire a redundant ignore-cursor-events command.
                void applyClickThroughRef.current?.(false);
                growForDropdown(state.slots.length);
              } else {
                // Root returns to content-sizing on re-render; re-fit on the
                // next frame (the ResizeObserver also catches the change).
                // Click-through stays OFF until the next poll re-evaluates from
                // cursor position, avoiding a flicker if the cursor is on header.
                requestAnimationFrame(() => fitToContent());
              }
            }}
          />
        </div>
        <div className="flex items-center gap-1.5" data-tauri-drag-region>
          <span data-tauri-drag-region>
            <StatusDot status={state.status} />
          </span>
          <button
            onClick={handleClose}
            data-tauri-drag-region="false"
            className="rounded p-0.5 text-muted-foreground/60 hover:bg-muted hover:text-foreground transition-colors"
            aria-label="Hide overlay"
          >
            <X className="h-3 w-3" />
          </button>
        </div>
      </header>

      {/* Body — content or error or placeholder. Click-through region.
          Type at 12 px / leading-relaxed (≈ 1.625) for comfortable Markdown
          rendering at the small overlay scale. `min-h-0` lets the flex child
          shrink so the card hugs its content. `overflow-y-auto` only ever
          engages past MAX_FIT_HEIGHT (rare — the engine caps output ~12 lines);
          in normal use the window fits the content and nothing scrolls. */}
      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-1.5 text-[12px] leading-relaxed">
        <Body state={state} />
      </div>
    </div>
  );
}

function StatusDot({ status }: { status: InsightStateEvent["status"] }) {
  if (status === "running") {
    return <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />;
  }
  if (status === "error") {
    return <AlertTriangle className="h-3 w-3 text-destructive" />;
  }
  return <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/40" />;
}

function Body({ state }: { state: InsightStateEvent }) {
  if (state.status === "error" && state.error) {
    return (
      <div className="flex items-start gap-1.5 text-destructive">
        <AlertTriangle className="h-3 w-3 shrink-0 mt-0.5" />
        <span className="leading-snug">{state.error}</span>
      </div>
    );
  }
  if (!state.content) {
    return (
      <div className="text-muted-foreground/70 italic">
        Waiting for the first result…
      </div>
    );
  }
  // markdownToBasicHtml runs the output through DOMPurify before returning,
  // so the HTML payload is already safe to inject. The `markdown-overlay`
  // modifier class adds overlay-specific spacing tweaks defined in
  // `index.css` (tighter list padding, no first/last child margin).
  return (
    <div
      className="ai-chat-markdown markdown-overlay"
      dangerouslySetInnerHTML={{ __html: markdownToBasicHtml(state.content) }}
    />
  );
}
