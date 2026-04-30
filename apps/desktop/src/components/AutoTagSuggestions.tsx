import { Check, X, Folder, Sparkles } from "lucide-react";
import type { FolderSuggestion } from "@/lib/auto-tag";

interface AutoTagSuggestionsProps {
  suggestions: FolderSuggestion[];
  onAccept: (suggestion: FolderSuggestion) => void;
  onDismiss: (suggestion: FolderSuggestion) => void;
}

export function AutoTagSuggestions({
  suggestions,
  onAccept,
  onDismiss,
}: AutoTagSuggestionsProps) {
  if (suggestions.length === 0) return null;

  return (
    <div className="flex flex-wrap items-center gap-1.5 px-4 py-1.5 border-b bg-muted/30">
      <span className="inline-flex items-center gap-1 text-[11px] uppercase tracking-wide text-muted-foreground">
        <Sparkles className="h-3 w-3" />
        Recommended
      </span>
      {suggestions.map((s) => {
        const pct = Math.round(s.confidence * 100);
        return (
        <div
          key={s.id}
          className="flex items-center gap-1 rounded-full border bg-background px-2 py-0.5 text-xs shadow-sm"
          title={`Confidence: ${pct}%`}
        >
          <Folder
            className="h-3 w-3 shrink-0"
            style={s.color ? { color: s.color } : undefined}
          />
          <span className="text-foreground">{s.name}</span>
          <span
            className={
              s.confidenceLevel === "high"
                ? "text-[10px] font-medium text-emerald-600 dark:text-emerald-400"
                : "text-[10px] font-medium text-muted-foreground"
            }
          >
            {pct}%
          </span>
          <button
            className="rounded-full p-0.5 text-muted-foreground hover:bg-green-500/15 hover:text-green-600 transition-colors"
            onClick={() => onAccept(s)}
            aria-label={`Add to ${s.name}`}
          >
            <Check className="h-3 w-3" />
          </button>
          <button
            className="rounded-full p-0.5 text-muted-foreground hover:bg-destructive/15 hover:text-destructive transition-colors"
            onClick={() => onDismiss(s)}
            aria-label={`Dismiss ${s.name}`}
          >
            <X className="h-3 w-3" />
          </button>
        </div>
        );
      })}
    </div>
  );
}
