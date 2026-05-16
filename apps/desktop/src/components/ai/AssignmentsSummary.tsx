import { useAppStore } from "@/stores/appStore";
import { MessageSquare, Sparkles } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { ProfilePicker } from "./ProfilePicker";

const ROWS: { key: "chat" | "aiActions"; label: string; icon: LucideIcon; helper: string }[] = [
  {
    key: "chat",
    label: "Chat",
    icon: MessageSquare,
    helper: "Default Profile for new chat conversations.",
  },
  {
    key: "aiActions",
    label: "AI actions",
    icon: Sparkles,
    helper: "Profile used by Summarize, Key Points, Action Items, Meeting Minutes.",
  },
];

export function AssignmentsSummary() {
  const aiConfig = useAppStore((s) => s.settings.aiConfig);
  const updateSettings = useAppStore((s) => s.updateSettings);

  function setAssignment(key: "chat" | "aiActions", next: string | null) {
    updateSettings({
      aiConfig: {
        ...aiConfig,
        assignments: {
          ...aiConfig.assignments,
          ...(key === "chat"
            ? { chatProfileId: next }
            : { aiActionsProfileId: next }),
        },
      },
    });
  }

  return (
    <div className="rounded-md border border-border bg-card">
      <div className="border-b border-border px-3 py-2.5">
        <h3 className="text-sm font-medium">Assignments</h3>
        <p className="mt-0.5 text-[11px] text-muted-foreground">
          Which Profile each AI feature uses by default.
        </p>
      </div>
      <div className="divide-y divide-border">
        {ROWS.map((row) => {
          const value =
            row.key === "chat"
              ? aiConfig.assignments.chatProfileId
              : aiConfig.assignments.aiActionsProfileId;
          const Icon = row.icon;
          return (
            <div
              key={row.key}
              className="grid grid-cols-[auto,1fr,minmax(180px,260px)] items-center gap-3 px-3 py-2.5"
            >
              <Icon className="h-4 w-4 text-muted-foreground" />
              <div className="min-w-0">
                <div className="text-sm font-medium">{row.label}</div>
                {value === null ? (
                  <div className="mt-0.5 text-[11px] text-muted-foreground">
                    No profile assigned — AI feature disabled.
                  </div>
                ) : (
                  <div className="mt-0.5 text-[11px] text-muted-foreground">
                    {row.helper}
                  </div>
                )}
              </div>
              <ProfilePicker
                profiles={aiConfig.profiles}
                connections={aiConfig.connections}
                value={value}
                onChange={(next) => setAssignment(row.key, next)}
                allowNone
                noneLabel="None — disable for this feature"
                variant="inline"
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
