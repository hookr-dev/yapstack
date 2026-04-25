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

  // Clear selection when the session being viewed changes.
  useEffect(() => {
    clearSegmentSelection();
  }, [sessionId, clearSegmentSelection]);

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

  // Marquee selection state
  const marqueeStartRef = useRef<{ x: number; y: number } | null>(null);
  const [marquee, setMarquee] = useState<{
    left: number;
    top: number;
    width: number;
    height: number;
  } | null>(null);

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
    if (currentPlaybackTime == null) {
      setUserScrolled(false);
    }
  }, [currentPlaybackTime]);

  // Auto-scroll to the active segment during playback (unless the user is freed).
  useEffect(() => {
    if (currentPlaybackTime != null && activeRef.current && !userScrolled) {
      autoScrollingRef.current = true;
      activeRef.current.scrollIntoView({ behavior: "smooth", block: "nearest" });
      setTimeout(() => {
        autoScrollingRef.current = false;
      }, 150);
    }
  }, [currentPlaybackTime, userScrolled]);

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

      if (currentPlaybackTime != null && activeRef.current) {
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPlaybackTime != null]);

  const handleJumpToCurrent = useCallback(() => {
    setUserScrolled(false);
    requestAnimationFrame(() => {
      if (currentPlaybackTime != null && activeRef.current) {
        activeRef.current.scrollIntoView({
          behavior: "smooth",
          block: "center",
        });
      } else {
        bottomRef.current?.scrollIntoView({ behavior: "smooth" });
      }
    });
  }, [currentPlaybackTime]);

  const handleTimestampClick = useCallback(
    (time: number) => {
      setUserScrolled(false);
      onTimestampClick?.(time);
    },
    [onTimestampClick]
  );

  const handleContainerPointerDown = (e: React.PointerEvent) => {
    // Only start marquee when the user clicks empty transcript space, not a
    // segment bubble. Segments have `data-segment-id` on their wrapper.
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    if (target.closest("[data-segment-id]")) return;
    // Block native text-selection (the blue highlight) during marquee —
    // the drag is ours, not the browser's.
    e.preventDefault();
    const container = e.currentTarget as HTMLElement;
    const rect = container.getBoundingClientRect();
    marqueeStartRef.current = {
      x: e.clientX - rect.left + container.scrollLeft,
      y: e.clientY - rect.top + container.scrollTop,
    };
    if (!(e.shiftKey || e.metaKey || e.ctrlKey)) {
      clearSegmentSelection();
    }
  };

  const handleContainerPointerMove = (e: React.PointerEvent) => {
    const start = marqueeStartRef.current;
    if (!start) return;
    // Clear any stray native selection the browser may have kicked off
    // before our preventDefault landed.
    window.getSelection()?.removeAllRanges();
    const container = e.currentTarget as HTMLElement;
    const rect = container.getBoundingClientRect();
    const x = e.clientX - rect.left + container.scrollLeft;
    const y = e.clientY - rect.top + container.scrollTop;
    const left = Math.min(start.x, x);
    const top = Math.min(start.y, y);
    const width = Math.abs(x - start.x);
    const height = Math.abs(y - start.y);
    if (width < 4 && height < 4) return;
    setMarquee({ left, top, width, height });
  };

  const handleContainerPointerUp = (e: React.PointerEvent) => {
    const start = marqueeStartRef.current;
    marqueeStartRef.current = null;
    if (!start || !marquee) {
      setMarquee(null);
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

  // Find the active segment based on playback time
  const activeSegmentId = useMemo(() => {
    if (currentPlaybackTime == null) return null;
    for (let i = segments.length - 1; i >= 0; i--) {
      if (segments[i].audio_offset_seconds <= currentPlaybackTime) {
        return segments[i].id;
      }
    }
    return null;
  }, [segments, currentPlaybackTime]);

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
              ? "relative space-y-2 px-3 py-2 select-none"
              : "relative space-y-2 px-3 py-2"
          }
          onPointerDown={handleContainerPointerDown}
          onPointerMove={handleContainerPointerMove}
          onPointerUp={handleContainerPointerUp}
          onPointerLeave={handleContainerPointerUp}
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
      {userScrolled && (
        <button
          onClick={handleJumpToCurrent}
          className="absolute bottom-3 left-1/2 -translate-x-1/2 z-10 flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-primary text-primary-foreground rounded-full shadow-md hover:bg-primary/90 transition-colors"
        >
          {scrollDirection === "up" ? (
            <ArrowUp className="h-3 w-3" />
          ) : (
            <ArrowDown className="h-3 w-3" />
          )}
          {currentPlaybackTime != null ? "Jump to current" : "Jump to latest"}
        </button>
      )}
    </div>
  );
}
