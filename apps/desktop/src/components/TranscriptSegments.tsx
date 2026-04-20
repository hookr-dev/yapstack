import { Fragment, useState, type RefObject } from "react";
import { EditableSegment } from "@/components/EditableSegment";
import { useAppStore } from "@/stores/appStore";
import type { DbSegment } from "@/lib/db";
import { Pencil } from "lucide-react";
import { Input } from "@/components/ui/input";

/// Up to 4 speakers (Sortformer cap). Subtle accent backgrounds tuned to
/// stay readable in both light and dark themes.
const SPEAKER_COLORS = [
  "bg-sky-500/10 text-sky-700 dark:text-sky-300 border-sky-500/30",
  "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 border-emerald-500/30",
  "bg-amber-500/10 text-amber-700 dark:text-amber-300 border-amber-500/30",
  "bg-violet-500/10 text-violet-700 dark:text-violet-300 border-violet-500/30",
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
  onTimestampClick?: (time: number) => void;
}

/// Renders a flat or speaker-grouped transcript depending on whether any
/// segment carries a `speaker_id`. Falls back to the existing flat layout
/// when no speakers are present, so Whisper sessions render unchanged.
export function TranscriptSegments({
  sessionId,
  segments,
  isEditable,
  activeSegmentId,
  activeRef,
  onTimestampClick,
}: TranscriptSegmentsProps) {
  const speakerNames = useAppStore(
    (s) => s.settings.speakerNames[sessionId],
  );

  const hasSpeakers = segments.some((s) => s.speaker_id != null);
  if (!hasSpeakers) {
    return (
      <>
        {segments.map((segment) => {
          const isActive = segment.id === activeSegmentId;
          return (
            <EditableSegment
              key={segment.id}
              segment={segment}
              isActive={isActive}
              readOnly={!isEditable}
              onTimestampClick={onTimestampClick}
              ref={isActive ? activeRef : undefined}
            />
          );
        })}
      </>
    );
  }

  // Group consecutive same-speaker segments. A null/undefined speaker_id
  // groups separately from any numbered speaker.
  type Group = { speakerId: number | null; items: DbSegment[] };
  const groups: Group[] = [];
  for (const seg of segments) {
    const id = seg.speaker_id ?? null;
    const tail = groups[groups.length - 1];
    if (tail && tail.speakerId === id) {
      tail.items.push(seg);
    } else {
      groups.push({ speakerId: id, items: [seg] });
    }
  }

  return (
    <>
      {groups.map((group, idx) => (
        <Fragment key={`${group.speakerId ?? "none"}-${idx}`}>
          {group.speakerId != null && (
            <SpeakerHeader
              sessionId={sessionId}
              speakerId={group.speakerId}
              customName={speakerNames?.[group.speakerId]}
            />
          )}
          {group.items.map((segment) => {
            const isActive = segment.id === activeSegmentId;
            return (
              <EditableSegment
                key={segment.id}
                segment={segment}
                isActive={isActive}
                readOnly={!isEditable}
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
