import { useState, useRef, useEffect, forwardRef, memo } from "react";
import { useAppStore } from "@/stores/appStore";
import type { DbSegment } from "@/lib/db";
import { cn, formatTime } from "@/lib/utils";
import { trackSegmentEdited, trackSegmentHidden } from "@/lib/analytics";
import { BookmarkPlus, Copy, Eye, EyeOff, Pencil, Trash2 } from "lucide-react";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

export const EditableSegment = memo(forwardRef<
  HTMLDivElement,
  {
    segment: DbSegment;
    isActive?: boolean;
    isSelected?: boolean;
    selectionActive?: boolean;
    readOnly?: boolean;
    orderedIds?: string[];
    onTimestampClick?: (time: number) => void;
  }
>(function EditableSegment(
  {
    segment,
    isActive,
    isSelected,
    selectionActive,
    readOnly,
    orderedIds,
    onTimestampClick,
  },
  ref,
) {
  const editSegmentText = useAppStore((s) => s.editSegmentText);
  const deleteSegment = useAppStore((s) => s.deleteSegment);
  const toggleSegmentHidden = useAppStore((s) => s.toggleSegmentHidden);
  const toggleSegmentSelected = useAppStore((s) => s.toggleSegmentSelected);
  const clearSegmentSelection = useAppStore((s) => s.clearSegmentSelection);

  const [isEditing, setIsEditing] = useState(false);
  const bubbleRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (isEditing && bubbleRef.current) {
      bubbleRef.current.focus();
      const range = document.createRange();
      const sel = window.getSelection();
      range.selectNodeContents(bubbleRef.current);
      range.collapse(false);
      sel?.removeAllRanges();
      sel?.addRange(range);
    }
  }, [isEditing]);

  const text = segment.text.trim();
  if (!text) return null;

  const isHidden = segment.hidden === 1;

  const isMic = segment.source === "Mic";
  const time = formatTime(Math.max(0, segment.audio_offset_seconds));
  const isLowConfidence = segment.confidence < 0.5;
  const isEdited = segment.edited_at != null;

  const handleSave = () => {
    if (!bubbleRef.current) return;
    const trimmed = (bubbleRef.current.textContent ?? "").trim();
    if (trimmed && trimmed !== segment.text) {
      editSegmentText(segment.id, trimmed);
      trackSegmentEdited();
    }
    setIsEditing(false);
  };

  const handleStartEdit = () => {
    setIsEditing(true);
  };

  // Shift-click extends the native text selection on mousedown, *before*
  // our click handler runs — that's what paints the blue highlight across
  // bubbles. Cancel that at the mousedown stage when a modifier is held.
  // (Plain mousedown is left alone so contenteditable focus still works.)
  const handleBubbleMouseDown = (e: React.MouseEvent) => {
    if (isEditing) return;
    if (e.shiftKey || e.metaKey || e.ctrlKey) {
      e.preventDefault();
      window.getSelection()?.removeAllRanges();
    }
  };

  const handleBubbleClick = (e: React.MouseEvent) => {
    if (isEditing) return;
    const isRange = e.shiftKey;
    const isToggle = e.metaKey || e.ctrlKey;
    if (isRange || isToggle) {
      e.preventDefault();
      window.getSelection()?.removeAllRanges();
      toggleSegmentSelected(
        segment.id,
        isRange ? "range" : "toggle",
        orderedIds ?? [],
      );
      return;
    }
    if (selectionActive) {
      // Bare click with an active selection: clear it, don't enter edit
      // mode on a segment the user might have been trying to deselect away
      // from. Second click enters edit mode.
      clearSegmentSelection();
      return;
    }
    if (!readOnly) handleStartEdit();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSave();
    }
    if (e.key === "Escape") {
      if (bubbleRef.current) {
        bubbleRef.current.textContent = segment.text;
      }
      setIsEditing(false);
    }
  };

  const handleCopy = () => {
    navigator.clipboard.writeText(segment.text);
  };

  const handleInsertIntoNotes = () => {
    window.dispatchEvent(
      new CustomEvent("yapstack:insert-segment-ref", {
        detail: {
          segmentId: segment.id,
          timestamp: formatTime(Math.max(0, segment.audio_offset_seconds)),
          offsetSeconds: segment.audio_offset_seconds,
        },
      }),
    );
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          ref={ref}
          data-segment-id={segment.id}
          className={cn(
            "flex",
            isMic ? "justify-end" : "justify-start",
            isHidden && "opacity-60",
          )}
        >
          <div className="max-w-[80%] space-y-0.5">
            <div
              ref={bubbleRef}
              contentEditable={isEditing}
              suppressContentEditableWarning
              className={cn(
                "rounded-2xl px-2.5 py-1.5 text-xs leading-relaxed whitespace-pre-wrap transition-colors",
                isMic
                  ? "bg-primary text-primary-foreground rounded-br-md"
                  : "bg-muted text-foreground rounded-bl-md",
                isLowConfidence && "opacity-60",
                isActive && "ring-2 shadow-md scale-[1.02]",
                isActive && (isMic
                  ? "ring-primary ring-offset-2 ring-offset-background"
                  : "ring-ring"),
                isSelected && "ring-2 shadow-sm",
                isSelected && (isMic
                  ? "ring-primary/60 ring-offset-2 ring-offset-background"
                  : "ring-primary/60"),
                isEditing
                  ? "outline-none ring-2 ring-ring cursor-text"
                  : readOnly
                    ? "cursor-default"
                    : "cursor-pointer",
              )}
              onMouseDown={!isEditing ? handleBubbleMouseDown : undefined}
              onClick={!isEditing ? handleBubbleClick : undefined}
              onBlur={isEditing ? handleSave : undefined}
              onKeyDown={isEditing ? handleKeyDown : undefined}
            >
              {text}
            </div>
            <div
              className={cn(
                "flex items-center gap-1 text-[9px] text-muted-foreground/60",
                isMic ? "justify-end" : "justify-start",
              )}
            >
              <span
                className={cn(
                  onTimestampClick && "cursor-pointer hover:text-foreground",
                )}
                onClick={
                  onTimestampClick
                    ? () => onTimestampClick(segment.audio_offset_seconds)
                    : undefined
                }
              >
                {time}
              </span>
              {isHidden && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <EyeOff className="h-2.5 w-2.5" aria-label="Hidden from AI and exports" />
                  </TooltipTrigger>
                  <TooltipContent>Hidden from AI and exports</TooltipContent>
                </Tooltip>
              )}
              {isEdited && <span>&middot; edited</span>}
            </div>
          </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        {!readOnly && (
          <ContextMenuItem onClick={handleStartEdit}>
            <Pencil />
            Edit
          </ContextMenuItem>
        )}
        <ContextMenuItem onClick={handleCopy}>
          <Copy />
          Copy
        </ContextMenuItem>
        <ContextMenuItem onClick={handleInsertIntoNotes}>
          <BookmarkPlus />
          Insert into Notes
        </ContextMenuItem>
        {!readOnly && (
          <>
            <ContextMenuItem onClick={() => { toggleSegmentHidden(segment.id); trackSegmentHidden(); }}>
              {isHidden ? (
                <Eye />
              ) : (
                <EyeOff />
              )}
              {isHidden ? "Unhide" : "Hide"}
            </ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem
              className="text-destructive"
              onClick={() => deleteSegment(segment.id)}
            >
              <Trash2 />
              Delete
            </ContextMenuItem>
          </>
        )}
      </ContextMenuContent>
    </ContextMenu>
  );
}));
