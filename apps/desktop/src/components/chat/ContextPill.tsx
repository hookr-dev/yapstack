import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

interface ContextPillProps {
  enabled: boolean;
  onToggle?: () => void;
  icon: ReactNode;
  label: string;
  /** Muted suffix appended after the label, e.g. "• 6 selected". */
  suffix?: string;
}

export function ContextPill({
  enabled,
  onToggle,
  icon,
  label,
  suffix,
}: ContextPillProps) {
  const content = (
    <>
      {icon}
      {label}
      {suffix && (
        <span className="text-muted-foreground/70">{suffix}</span>
      )}
    </>
  );

  if (!onToggle) {
    return (
      <span className="inline-flex items-center gap-1 rounded-md border border-primary/30 bg-primary/5 px-2 py-0.5 text-[9px] text-foreground">
        {content}
      </span>
    );
  }

  return (
    <button
      onClick={onToggle}
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-[9px] transition-colors",
        enabled
          ? "border-primary/30 bg-primary/5 text-foreground"
          : "border-muted-foreground/20 bg-transparent text-muted-foreground/50",
      )}
    >
      {content}
    </button>
  );
}
