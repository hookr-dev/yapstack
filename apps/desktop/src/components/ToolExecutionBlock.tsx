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
} from "lucide-react";
import type { ToolExecution } from "@/lib/ai";

const TOOL_ICONS: Record<string, typeof Check> = {
  add_session_to_folder: FolderInput,
  search_folders: Search,
  search_sessions: Search,
  get_session_context: FileText,
  search_dictations: Mic,
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
    <div className="flex flex-col gap-0.5 mb-1.5">
      {executions.map((exec, i) => (
        <ToolExecutionRow key={`${exec.name}-${i}`} exec={exec} />
      ))}
    </div>
  );
}

function ToolExecutionRow({ exec }: { exec: ToolExecution }) {
  const Icon = TOOL_ICONS[exec.name] ?? FileText;

  return (
    <div className="flex items-center gap-1.5 text-[11px] leading-tight">
      <span className="flex items-center justify-center w-4 h-4 shrink-0">
        {exec.status === "running" ? (
          <Loader2 className="h-3 w-3 text-primary animate-spin" />
        ) : exec.status === "error" ? (
          <AlertCircle className="h-3 w-3 text-destructive" />
        ) : (
          <Check className="h-3 w-3 text-primary" />
        )}
      </span>
      <Icon className="h-3 w-3 shrink-0 text-muted-foreground" />
      <span
        className={
          exec.status === "running"
            ? "text-foreground/80"
            : exec.status === "error"
              ? "text-destructive/80"
              : "text-muted-foreground"
        }
      >
        {exec.label}
      </span>
      {exec.detail && exec.status !== "running" && (
        <span className="text-muted-foreground/60 truncate max-w-[200px]">
          — {exec.detail}
        </span>
      )}
    </div>
  );
}
