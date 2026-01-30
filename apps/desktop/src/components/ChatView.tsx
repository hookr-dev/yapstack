import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, ArrowUp } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { EditableSegment } from "@/components/EditableSegment";
import type { DbSegment } from "@/lib/db";

export function ChatView({
  segments,
  backfillActive,
  isEditable,
  currentPlaybackTime,
  onTimestampClick,
  initialScrollToBottom,
}: {
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
  const [userScrolled, setUserScrolled] = useState(false);
  const [scrollDirection, setScrollDirection] = useState<"up" | "down">("down");

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [segments.length]);

  // On mount, scroll to bottom for active sessions so user sees latest segments
  useEffect(() => {
    if (initialScrollToBottom) {
      requestAnimationFrame(() => {
        bottomRef.current?.scrollIntoView({ behavior: "instant" });
      });
    }
    // Only run on mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Reset userScrolled when playback stops
  useEffect(() => {
    if (currentPlaybackTime == null) {
      setUserScrolled(false);
    }
  }, [currentPlaybackTime]);

  // Auto-scroll to active segment during playback (unless user scrolled away)
  useEffect(() => {
    if (currentPlaybackTime != null && activeRef.current && !userScrolled) {
      autoScrollingRef.current = true;
      activeRef.current.scrollIntoView({ behavior: "smooth", block: "nearest" });
      // Clear flag after scroll settles
      setTimeout(() => {
        autoScrollingRef.current = false;
      }, 150);
    }
  }, [currentPlaybackTime, userScrolled]);

  // Detect user scroll vs programmatic scroll
  useEffect(() => {
    const viewport = scrollAreaRef.current?.querySelector(
      "[data-slot='scroll-area-viewport']"
    );
    if (!viewport || currentPlaybackTime == null) return;

    const handleScroll = () => {
      if (autoScrollingRef.current) return;
      if (activeRef.current) {
        const rect = activeRef.current.getBoundingClientRect();
        const vpRect = viewport.getBoundingClientRect();
        const visible = rect.top >= vpRect.top && rect.bottom <= vpRect.bottom;
        setUserScrolled(!visible);
        if (!visible) {
          setScrollDirection(rect.top < vpRect.top ? "up" : "down");
        }
      }
    };

    viewport.addEventListener("scroll", handleScroll, { passive: true });
    return () => viewport.removeEventListener("scroll", handleScroll);
    // Only attach/detach when playback starts/stops
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPlaybackTime != null]);

  const handleJumpToCurrent = useCallback(() => {
    setUserScrolled(false);
    // Delay scrollIntoView so the autoscroll effect picks it up on next tick
    requestAnimationFrame(() => {
      activeRef.current?.scrollIntoView({ behavior: "smooth", block: "center" });
    });
  }, []);

  const handleTimestampClick = useCallback(
    (time: number) => {
      setUserScrolled(false);
      onTimestampClick?.(time);
    },
    [onTimestampClick]
  );

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
        <div className="space-y-2 px-3 py-2">
          {segments.map((segment) => {
            const isActive = segment.id === activeSegmentId;
            return (
              <EditableSegment
                key={segment.id}
                segment={segment}
                isActive={isActive}
                readOnly={!isEditable}
                onTimestampClick={handleTimestampClick}
                ref={isActive ? activeRef : undefined}
              />
            );
          })}
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
      {userScrolled && currentPlaybackTime != null && (
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
