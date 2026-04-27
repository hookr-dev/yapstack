import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, ArrowUp } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { TranscriptSegments } from "@/components/TranscriptSegments";
import { BulkActionsBar } from "@/components/transcript/BulkActionsBar";
import { useAppStore } from "@/stores/appStore";
import type { DbSegment } from "@/lib/db";

export function ChatView({
  sessionId,
  segments,
  backfillActive,
  isEditable,
  currentPlaybackTime,
  onTimestampClick,
  initialScrollToBottom,
}: {
  /// Required for speaker-name rename persistence; falls back to "" when
  /// rendering ad-hoc segment lists outside a session.
  sessionId?: string;
  segments: DbSegment[];
  backfillActive?: boolean;
  isEditable?: boolean;
  currentPlaybackTime?: number;
  onTimestampClick?: (time: number) => void;
  initialScrollToBottom?: boolean;
}) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const activeRef = useRef<HTMLDivElement>(null);
  const scrollAreaRef = useRef<HTMLDivElement>(null);
  const autoScrollingRef = useRef(false);
  const lastScrollTopRef = useRef(0);
  const [userScrolled, setUserScrolled] = useState(false);
  const [scrollDirection, setScrollDirection] = useState<"up" | "down">("down");

  const selectedSegmentIds = useAppStore((s) => s.selectedSegmentIds);
  const setSegmentSelection = useAppStore((s) => s.setSegmentSelection);
  const clearSegmentSelection = useAppStore((s) => s.clearSegmentSelection);
  const deleteSegments = useAppStore((s) => s.deleteSegments);
  // `currentPlaybackTime` persists across sessions and is `0` even
  // when no audio has played, so it isn't a reliable "audio active"
  // signal — gate playback UI on `isPlaying` instead.
  const isPlaying = useAppStore((s) => s.isPlaying);
  const setPlaybackTime = useAppStore((s) => s.setPlaybackTime);
  const setIsPlaying = useAppStore((s) => s.setIsPlaying);

  // Playback state is global; reset on session switch so it doesn't
  // bleed across sessions.
  useEffect(() => {
    clearSegmentSelection();
    setPlaybackTime(0);
    setIsPlaying(false);
  }, [sessionId, clearSegmentSelection, setPlaybackTime, setIsPlaying]);

  // Selection-scoped keyboard shortcuts: Escape clears, Backspace/Delete
  // bulk-deletes. Only fires when the user isn't typing in an input and
  // there's an active selection.
  useEffect(() => {
    if (selectedSegmentIds.size === 0) return;
    const onKey = (e: KeyboardEvent) => {
      const el = document.activeElement as HTMLElement | null;
      if (
        el &&
        (el.tagName === "INPUT" ||
          el.tagName === "TEXTAREA" ||
          el.isContentEditable)
      ) {
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        clearSegmentSelection();
      } else if ((e.key === "Backspace" || e.key === "Delete") && isEditable) {
        e.preventDefault();
        deleteSegments(Array.from(selectedSegmentIds));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedSegmentIds, clearSegmentSelection, deleteSegments, isEditable]);

  const orderedIds = useMemo(() => segments.map((s) => s.id), [segments]);

  // Marquee selection state — drag-select on whitespace within the
  // transcript. Bubbles own their own click handling (incl. ContextMenu
  // right-click, shift-click range, cmd-click toggle), so the marquee
  // never starts on a bubble. This keeps the system simple: there's no
  // click-vs-drag disambiguation for bubbles to fight with.
  const marqueeStartRef = useRef<{ x: number; y: number } | null>(null);
  const [marquee, setMarquee] = useState<{
    left: number;
    top: number;
    width: number;
    height: number;
  } | null>(null);
  // Edge auto-scroll: 30 px hot zones at top + bottom, linear velocity
  // ramp capped at 12 px/frame, RAF loop that recomputes the marquee
  // end-point from the cached pointer each tick so the rect grows as
  // the viewport scrolls under a stationary cursor.
  const viewportRef = useRef<HTMLElement | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const scrollRafRef = useRef<number | null>(null);
  const containerRef = useRef<HTMLElement | null>(null);

  // Stick-to-bottom: follow new segments unless the user has scrolled away.
  useEffect(() => {
    if (userScrolled) return;
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [segments.length, userScrolled]);

  useEffect(() => {
    if (initialScrollToBottom) {
      requestAnimationFrame(() => {
        bottomRef.current?.scrollIntoView({ behavior: "instant" });
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // When playback stops, resume following so the next session sticks again.
  useEffect(() => {
    if (!isPlaying) {
      setUserScrolled(false);
    }
  }, [isPlaying]);

  // Auto-scroll to the active segment during playback (unless the user is freed).
  useEffect(() => {
    if (!isPlaying || !activeRef.current || userScrolled) return;
    autoScrollingRef.current = true;
    activeRef.current.scrollIntoView({ behavior: "smooth", block: "nearest" });
    // Cleanup matters here — the effect re-runs on every timeupdate
    // tick, so without it stale timers race with subsequent legit
    // scrolls and flip autoScrollingRef.current mid-gesture.
    const t = setTimeout(() => {
      autoScrollingRef.current = false;
    }, 150);
    return () => clearTimeout(t);
  }, [currentPlaybackTime, userScrolled, isPlaying]);

  // Disengage stick-to-bottom on user scroll. Programmatic autoscroll only
  // moves the viewport *down*, so any decrease in scrollTop is unambiguously
  // the user — robust against rapid segment arrival.
  useEffect(() => {
    const viewport = scrollAreaRef.current?.querySelector(
      "[data-slot='scroll-area-viewport']"
    ) as HTMLElement | null;
    if (!viewport) return;

    lastScrollTopRef.current = viewport.scrollTop;

    const handleScroll = () => {
      const newScrollTop = viewport.scrollTop;
      const wentUp = newScrollTop < lastScrollTopRef.current - 1;
      lastScrollTopRef.current = newScrollTop;

      if (isPlaying && activeRef.current) {
        if (autoScrollingRef.current) return;
        const rect = activeRef.current.getBoundingClientRect();
        const vpRect = viewport.getBoundingClientRect();
        const visible = rect.top >= vpRect.top && rect.bottom <= vpRect.bottom;
        setUserScrolled(!visible);
        if (!visible) {
          setScrollDirection(rect.top < vpRect.top ? "up" : "down");
        }
        return;
      }

      const distFromBottom =
        viewport.scrollHeight - newScrollTop - viewport.clientHeight;
      if (wentUp) {
        setUserScrolled(true);
        setScrollDirection("down");
      } else if (distFromBottom < 4) {
        setUserScrolled(false);
      }
    };

    viewport.addEventListener("scroll", handleScroll, { passive: true });
    return () => viewport.removeEventListener("scroll", handleScroll);
  }, [isPlaying]);

  const handleJumpToCurrent = useCallback(() => {
    setUserScrolled(false);
    requestAnimationFrame(() => {
      activeRef.current?.scrollIntoView({
        behavior: "smooth",
        block: "center",
      });
    });
  }, []);

  const handleTimestampClick = useCallback(
    (time: number) => {
      setUserScrolled(false);
      onTimestampClick?.(time);
    },
    [onTimestampClick]
  );

  const SCROLL_HOT_ZONE = 30; // px from viewport edge that triggers scroll
  const SCROLL_MAX_VELOCITY = 12; // px / frame at the very edge

  // Compute auto-scroll velocity from a pointer's clientY relative to
  // the viewport. Linear ramp inside the hot zones, zero otherwise.
  // Positive = scroll down, negative = scroll up.
  const computeScrollVelocity = (clientY: number, viewport: HTMLElement) => {
    const r = viewport.getBoundingClientRect();
    const distFromTop = clientY - r.top;
    const distFromBottom = r.bottom - clientY;
    if (distFromTop < SCROLL_HOT_ZONE && distFromTop >= 0) {
      const ramp = (SCROLL_HOT_ZONE - distFromTop) / SCROLL_HOT_ZONE;
      return -Math.min(SCROLL_MAX_VELOCITY, SCROLL_MAX_VELOCITY * ramp);
    }
    if (distFromBottom < SCROLL_HOT_ZONE && distFromBottom >= 0) {
      const ramp = (SCROLL_HOT_ZONE - distFromBottom) / SCROLL_HOT_ZONE;
      return Math.min(SCROLL_MAX_VELOCITY, SCROLL_MAX_VELOCITY * ramp);
    }
    // Outside the viewport entirely (pointer captured past the edge):
    // keep scrolling at max velocity in the appropriate direction.
    if (distFromTop < 0) return -SCROLL_MAX_VELOCITY;
    if (distFromBottom < 0) return SCROLL_MAX_VELOCITY;
    return 0;
  };

  // Recompute the marquee rect from the cached pointer position. Called
  // both on pointermove (cursor moved) and during the auto-scroll RAF
  // tick (viewport scrolled but cursor stationary).
  const updateMarqueeFromPointer = () => {
    const start = marqueeStartRef.current;
    const ptr = lastPointerRef.current;
    const container = containerRef.current;
    if (!start || !ptr || !container) return;
    const rect = container.getBoundingClientRect();
    const x = ptr.x - rect.left + container.scrollLeft;
    const y = ptr.y - rect.top + container.scrollTop;
    const left = Math.min(start.x, x);
    const top = Math.min(start.y, y);
    const width = Math.abs(x - start.x);
    const height = Math.abs(y - start.y);
    if (width < 4 && height < 4) return;
    setMarquee({ left, top, width, height });
  };

  const stopAutoScroll = () => {
    if (scrollRafRef.current != null) {
      cancelAnimationFrame(scrollRafRef.current);
      scrollRafRef.current = null;
    }
  };

  const tickAutoScroll = () => {
    const viewport = viewportRef.current;
    const ptr = lastPointerRef.current;
    if (!viewport || !ptr || !marqueeStartRef.current) {
      scrollRafRef.current = null;
      return;
    }
    const velocity = computeScrollVelocity(ptr.y, viewport);
    if (velocity === 0) {
      scrollRafRef.current = null;
      return;
    }
    const before = viewport.scrollTop;
    viewport.scrollTop = before + velocity;
    if (viewport.scrollTop !== before) updateMarqueeFromPointer();
    scrollRafRef.current = requestAnimationFrame(tickAutoScroll);
  };

  const ensureAutoScrollRunning = () => {
    if (scrollRafRef.current != null) return;
    if (!viewportRef.current || !lastPointerRef.current) return;
    if (computeScrollVelocity(lastPointerRef.current.y, viewportRef.current) === 0) return;
    scrollRafRef.current = requestAnimationFrame(tickAutoScroll);
  };

  // Cancel any in-flight RAF when the component unmounts mid-drag.
  useEffect(() => {
    return () => stopAutoScroll();
  }, []);

  const handleContainerPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    // Bubbles handle their own clicks (incl. shift/cmd modifiers) and
    // right-click ContextMenu; the marquee only starts on whitespace.
    // Drag-select bubbles via shift-click instead.
    if (target.closest("[data-segment-id]")) return;
    // preventDefault below blocks the focus shift the click would
    // otherwise cause; explicitly blur so an editing bubble's onBlur
    // still fires and saves.
    const active = document.activeElement as HTMLElement | null;
    if (active && active !== document.body) active.blur();
    // Pointer capture so pointermove fires past the viewport edge —
    // required for edge auto-scroll.
    e.currentTarget.setPointerCapture(e.pointerId);
    e.preventDefault();
    const container = e.currentTarget as HTMLElement;
    containerRef.current = container;
    // Cache the Radix ScrollArea viewport — that's the actual scrolling
    // node, not our marquee container.
    viewportRef.current =
      scrollAreaRef.current?.querySelector<HTMLElement>(
        '[data-slot="scroll-area-viewport"]',
      ) ?? null;
    const rect = container.getBoundingClientRect();
    marqueeStartRef.current = {
      x: e.clientX - rect.left + container.scrollLeft,
      y: e.clientY - rect.top + container.scrollTop,
    };
    lastPointerRef.current = { x: e.clientX, y: e.clientY };
  };

  const handleContainerPointerMove = (e: React.PointerEvent) => {
    if (!marqueeStartRef.current) return;
    window.getSelection()?.removeAllRanges();
    lastPointerRef.current = { x: e.clientX, y: e.clientY };
    updateMarqueeFromPointer();
    ensureAutoScrollRunning();
  };

  const handleContainerPointerUp = (e: React.PointerEvent) => {
    stopAutoScroll();
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    const start = marqueeStartRef.current;
    marqueeStartRef.current = null;
    lastPointerRef.current = null;
    viewportRef.current = null;
    containerRef.current = null;
    if (!start || !marquee) {
      setMarquee(null);
      // Bare click on whitespace (no drag) clears the multi-selection.
      // Pointerdown only fires on whitespace (we bail on bubbles), so
      // every no-drag pointerup here was a whitespace click.
      if (start) clearSegmentSelection();
      return;
    }
    const container = e.currentTarget as HTMLElement;
    const containerRect = container.getBoundingClientRect();
    const marqueeRect = {
      left: marquee.left - container.scrollLeft + containerRect.left,
      top: marquee.top - container.scrollTop + containerRect.top,
      right:
        marquee.left + marquee.width - container.scrollLeft + containerRect.left,
      bottom:
        marquee.top + marquee.height - container.scrollTop + containerRect.top,
    };
    const nodes = container.querySelectorAll<HTMLElement>("[data-segment-id]");
    const picked = new Set<string>();
    nodes.forEach((node) => {
      const r = node.getBoundingClientRect();
      const intersects =
        r.left < marqueeRect.right &&
        r.right > marqueeRect.left &&
        r.top < marqueeRect.bottom &&
        r.bottom > marqueeRect.top;
      if (intersects) {
        const id = node.getAttribute("data-segment-id");
        if (id) picked.add(id);
      }
    });
    const additive = e.shiftKey || e.metaKey || e.ctrlKey;
    if (additive) {
      const merged = new Set(selectedSegmentIds);
      picked.forEach((id) => merged.add(id));
      setSegmentSelection(merged);
    } else {
      setSegmentSelection(picked);
    }
    setMarquee(null);
  };

  // Find the active segment based on playback time. Null when not
  // playing — `currentPlaybackTime` alone is unreliable (see above).
  const activeSegmentId = useMemo(() => {
    if (!isPlaying || currentPlaybackTime == null) return null;
    for (let i = segments.length - 1; i >= 0; i--) {
      if (segments[i].audio_offset_seconds <= currentPlaybackTime) {
        return segments[i].id;
      }
    }
    return null;
  }, [segments, currentPlaybackTime, isPlaying]);

  if (segments.length === 0 && !backfillActive) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <p className="text-sm text-muted-foreground">
          Start speaking to begin transcription
        </p>
      </div>
    );
  }

  return (
    <div className="relative min-h-0 flex-1 select-text">
      <ScrollArea ref={scrollAreaRef} className="h-full">
        <div
          className={
            marquee
              ? "relative min-h-full space-y-2 px-3 py-2 select-none"
              : "relative min-h-full space-y-2 px-3 py-2"
          }
          onPointerDown={handleContainerPointerDown}
          onPointerMove={handleContainerPointerMove}
          onPointerUp={handleContainerPointerUp}
          onPointerCancel={handleContainerPointerUp}
        >
          <TranscriptSegments
            sessionId={sessionId ?? ""}
            segments={segments}
            isEditable={!!isEditable}
            activeSegmentId={activeSegmentId}
            activeRef={activeRef}
            selectedSegmentIds={selectedSegmentIds}
            orderedIds={orderedIds}
            onTimestampClick={handleTimestampClick}
          />
          {marquee && (
            <div
              className="pointer-events-none absolute rounded border border-primary/50 bg-primary/10"
              style={{
                left: marquee.left,
                top: marquee.top,
                width: marquee.width,
                height: marquee.height,
              }}
            />
          )}
          {backfillActive && (
            <div className="flex items-center gap-4 py-2 text-muted-foreground">
              <div className="h-px flex-1 border-t border-dashed" />
              <span className="relative flex h-2 w-2">
                <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-primary opacity-75" />
                <span className="relative inline-flex h-2 w-2 rounded-full bg-primary" />
              </span>
              <span className="text-[11px] font-medium">Processing prior audio</span>
              <div className="h-px flex-1 border-t border-dashed" />
            </div>
          )}
          <div ref={bottomRef} />
        </div>
      </ScrollArea>
      <BulkActionsBar segments={segments} readOnly={!isEditable} />
      {userScrolled && isPlaying && (
        <button
          onClick={handleJumpToCurrent}
          className="absolute bottom-3 left-1/2 -translate-x-1/2 z-10 flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-primary text-primary-foreground rounded-full shadow-md hover:bg-primary/90 transition-colors"
        >
          {scrollDirection === "up" ? (
            <ArrowUp className="h-3 w-3" />
          ) : (
            <ArrowDown className="h-3 w-3" />
          )}
          Jump to current
        </button>
      )}
    </div>
  );
}
