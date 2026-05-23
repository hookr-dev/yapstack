import { useAppStore } from "@/stores/appStore";
import { Label } from "@/components/ui/label";
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
    helper: "Summarize, Key Points, Action Items, Meeting Minutes.",
  },
];

export function AssignmentsSection() {
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
    <div className="space-y-3">
      <div className="space-y-0.5">
        <h4 className="text-[11px] font-medium uppercase text-muted-foreground">
          Assignments
        </h4>
        <p className="text-[10px] text-muted-foreground">
          Which Profile each AI feature uses by default.
        </p>
      </div>
      <div className="space-y-3">
        {ROWS.map((row) => {
          const value =
            row.key === "chat"
              ? aiConfig.assignments.chatProfileId
              : aiConfig.assignments.aiActionsProfileId;
          const Icon = row.icon;
          return (
            <div
              key={row.key}
              className="flex items-center justify-between gap-3"
            >
              <div className="flex min-w-0 items-center gap-2">
                <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <div className="min-w-0 space-y-0.5">
                  <Label className="text-xs">{row.label}</Label>
                  <p className="text-[10px] text-muted-foreground">
                    {value === null
                      ? "No Profile assigned — AI feature disabled."
                      : row.helper}
                  </p>
                </div>
              </div>
              <ProfilePicker
                profiles={aiConfig.profiles}
                connections={aiConfig.connections}
                value={value}
                onChange={(next) => setAssignment(row.key, next)}
                allowNone
                noneLabel="None — disable for this feature"
                variant="pill"
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
