import { Fragment, useMemo, useState, type RefObject } from "react";
import { EditableSegment } from "@/components/EditableSegment";
import { useAppStore } from "@/stores/appStore";
import type { DbSegment } from "@/lib/db";
import { Pencil } from "lucide-react";
import { Input } from "@/components/ui/input";

const SPEAKER_COLORS = [
  "bg-teal-500/10 text-teal-700 dark:text-teal-300 border-teal-500/30",
  "bg-rose-500/10 text-rose-700 dark:text-rose-300 border-rose-500/30",
  "bg-amber-500/10 text-amber-700 dark:text-amber-300 border-amber-500/30",
  "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 border-emerald-500/30",
] as const;

function speakerColor(speakerId: number): string {
  return SPEAKER_COLORS[speakerId % SPEAKER_COLORS.length];
}

function defaultSpeakerLabel(speakerId: number): string {
  return `Speaker ${speakerId + 1}`;
}

interface SpeakerHeaderProps {
  sessionId: string;
  speakerId: number;
  customName: string | undefined;
}

function SpeakerHeader({ sessionId, speakerId, customName }: SpeakerHeaderProps) {
  const setSpeakerName = useAppStore((s) => s.setSpeakerName);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(customName ?? "");

  const display = customName ?? defaultSpeakerLabel(speakerId);

  const commit = () => {
    setSpeakerName(sessionId, speakerId, draft);
    setEditing(false);
  };

  if (editing) {
    return (
      <div className="flex items-center gap-1.5 pt-2">
        <Input
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") {
              setDraft(customName ?? "");
              setEditing(false);
            }
          }}
          placeholder={defaultSpeakerLabel(speakerId)}
          className="h-6 max-w-[140px] px-2 text-[11px]"
        />
      </div>
    );
  }

  return (
    <button
      type="button"
      onClick={() => {
        setDraft(customName ?? "");
        setEditing(true);
      }}
      className={`group flex items-center gap-1 rounded-md border px-2 py-0.5 text-[10px] font-medium ${speakerColor(
        speakerId,
      )} hover:opacity-90`}
    >
      <span>{display}</span>
      <Pencil className="h-2.5 w-2.5 opacity-0 group-hover:opacity-70" />
    </button>
  );
}

interface TranscriptSegmentsProps {
  sessionId: string;
  segments: DbSegment[];
  isEditable: boolean;
  activeSegmentId: string | null;
  activeRef?: RefObject<HTMLDivElement | null>;
  selectedSegmentIds?: Set<string>;
  orderedIds?: string[];
  onTimestampClick?: (time: number) => void;
}

type Group = {
  source: "Mic" | "System";
  speakerId: number | null;
  items: DbSegment[];
};

export function TranscriptSegments({
  sessionId,
  segments,
  isEditable,
  activeSegmentId,
  activeRef,
  selectedSegmentIds,
  orderedIds,
  onTimestampClick,
}: TranscriptSegmentsProps) {
  const selectionActive = (selectedSegmentIds?.size ?? 0) > 0;
  const speakerNames = useAppStore(
    (s) => s.settings.speakerNames[sessionId],
  );

  // Source-vs-source visual differentiation lives in `EditableSegment`
  // (chat-bubble alignment / styling), so we don't add per-group "You" /
  // "Other" headers here. Headers are reserved for Sortformer speakers
  // (renamable via `setSpeakerName`); when diarization is wired in, groups
  // with a `speaker_id` get a `SpeakerHeader`. With diarization off, the
  // transcript renders flat and bubbles handle the rest.
  const { groups, hasSpeakerIds } = useMemo(() => {
    let speakerSeen = false;
    for (const s of segments) {
      if (s.speaker_id != null) {
        speakerSeen = true;
        break;
      }
    }
    if (!speakerSeen) {
      return { groups: [] as Group[], hasSpeakerIds: false };
    }
    // New group on either source change or speaker change so two adjacent
    // same-id segments from different sources don't visually merge under
    // one SpeakerHeader.
    const out: Group[] = [];
    for (const seg of segments) {
      const speakerId = seg.speaker_id ?? null;
      const tail = out[out.length - 1];
      if (tail && tail.source === seg.source && tail.speakerId === speakerId) {
        tail.items.push(seg);
      } else {
        out.push({ source: seg.source, speakerId, items: [seg] });
      }
    }
    return { groups: out, hasSpeakerIds: true };
  }, [segments]);

  if (!hasSpeakerIds) {
    return segments.map((segment) => {
      const isActive = segment.id === activeSegmentId;
      const isSelected = selectedSegmentIds?.has(segment.id) ?? false;
      return (
        <EditableSegment
          key={segment.id}
          segment={segment}
          isActive={isActive}
          isSelected={isSelected}
          selectionActive={selectionActive}
          readOnly={!isEditable}
          orderedIds={orderedIds}
          onTimestampClick={onTimestampClick}
          ref={isActive ? activeRef : undefined}
        />
      );
    });
  }

  return (
    <>
      {groups.map((group, idx) => (
        <Fragment key={idx}>
          {group.speakerId != null && (
            <SpeakerHeader
              sessionId={sessionId}
              speakerId={group.speakerId}
              customName={speakerNames?.[group.speakerId]}
            />
          )}
          {group.items.map((segment) => {
            const isActive = segment.id === activeSegmentId;
            const isSelected = selectedSegmentIds?.has(segment.id) ?? false;
            return (
              <EditableSegment
                key={segment.id}
                segment={segment}
                isActive={isActive}
                isSelected={isSelected}
                selectionActive={selectionActive}
                readOnly={!isEditable}
                orderedIds={orderedIds}
                onTimestampClick={onTimestampClick}
                ref={isActive ? activeRef : undefined}
              />
            );
          })}
        </Fragment>
      ))}
    </>
  );
}
