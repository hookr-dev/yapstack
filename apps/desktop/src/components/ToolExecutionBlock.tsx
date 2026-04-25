import {
  Check,
  Loader2,
  AlertCircle,
  FolderInput,
  Tag,
  FileText,
  Type,
  Pin,
  Search,
  Mic,
  Pencil,
  Undo2,
} from "lucide-react";
import type { ToolExecution } from "@/lib/ai";

const TOOL_ICONS: Record<string, typeof Check> = {
  add_session_to_folder: FolderInput,
  search_folders: Search,
  search_sessions: Search,
  get_session_context: FileText,
  search_dictations: Mic,
  replace_in_transcript: Pencil,
  tag_session: Tag,
  save_to_notes: FileText,
  update_title: Type,
  pin_session: Pin,
};

interface ToolExecutionBlockProps {
  executions: ToolExecution[];
}

export function ToolExecutionBlock({ executions }: ToolExecutionBlockProps) {
  if (executions.length === 0) return null;

  return (
    <div className="flex flex-col gap-0.5">
      {executions.map((exec, i) => (
        <ToolExecutionRow key={`${exec.name}-${i}`} exec={exec} />
      ))}
    </div>
  );
}

function ToolExecutionRow({ exec }: { exec: ToolExecution }) {
  const Icon = TOOL_ICONS[exec.name] ?? FileText;
  const undone = exec.undone === true;

  // Status icon: undone wins over status — even an erroring call that
  // ended up undone should read as "reverted, no longer in effect."
  const statusIcon = undone ? (
    <Undo2 className="h-3 w-3 text-muted-foreground/60" />
  ) : exec.status === "running" ? (
    <Loader2 className="h-3 w-3 text-primary animate-spin" />
  ) : exec.status === "error" ? (
    <AlertCircle className="h-3 w-3 text-destructive" />
  ) : (
    <Check className="h-3 w-3 text-primary" />
  );

  const labelClass = undone
    ? "text-muted-foreground/50 line-through"
    : exec.status === "running"
      ? "text-foreground/80"
      : exec.status === "error"
        ? "text-destructive/80"
        : "text-muted-foreground";

  const detailClass = undone
    ? "text-muted-foreground/40 line-through truncate max-w-[200px]"
    : "text-muted-foreground/60 truncate max-w-[200px]";

  return (
    <div
      className="flex items-center gap-1.5 text-[11px] leading-tight"
      title={undone ? "Undone by user" : undefined}
    >
      <span className="flex items-center justify-center w-4 h-4 shrink-0">
        {statusIcon}
      </span>
      <Icon
        className={`h-3 w-3 shrink-0 ${undone ? "text-muted-foreground/40" : "text-muted-foreground"}`}
      />
      <span className={labelClass}>{exec.label}</span>
      {exec.detail && exec.status !== "running" && (
        <span className={detailClass}>— {exec.detail}</span>
      )}
    </div>
  );
}
