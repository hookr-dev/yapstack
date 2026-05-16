import { useEffect, useMemo, useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Network, Pencil, Plus, Server, Sparkles, Trash2 } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type {
  AIConfig,
  AIProviderKind,
  Connection,
} from "@/lib/ai";
import { ConnectionEditorDialog } from "./ConnectionEditorDialog";
import {
  clearChatContextProfile,
  getChatContextProfileId,
} from "@/lib/db";

const KIND_ICON: Record<AIProviderKind, LucideIcon> = {
  openai: Sparkles,
  openrouter: Network,
  custom: Server,
};

const KIND_LABEL: Record<AIProviderKind, string> = {
  openai: "OpenAI",
  openrouter: "OpenRouter",
  custom: "Custom",
};

interface EditState {
  open: boolean;
  mode: "create" | "edit";
  initial?: Connection;
}

interface DeleteState {
  open: boolean;
  connection: Connection;
  dependentProfileIds: string[];
  dependentProfileNames: string[];
  affectedAssignments: string[]; // human-readable labels
  affectedSlotNames: string[];
}

export function ConnectionsSubTab({
  autoOpenEditor = false,
  onAutoOpenConsumed,
}: {
  autoOpenEditor?: boolean;
  onAutoOpenConsumed?: () => void;
} = {}) {
  const aiConfig = useAppStore((s) => s.settings.aiConfig);
  const dictation = useAppStore((s) => s.settings.dictation);
  const updateSettings = useAppStore((s) => s.updateSettings);

  const [editState, setEditState] = useState<EditState>({
    open: false,
    mode: "create",
  });
  const [deleteState, setDeleteState] = useState<DeleteState | null>(null);

  // Honor the one-shot autoOpenEditor signal exactly once on mount.
  useEffect(() => {
    if (autoOpenEditor) {
      setEditState({ open: true, mode: "create" });
      onAutoOpenConsumed?.();
    }
    // Intentional: only fire on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const connections = aiConfig.connections;

  const handleCreate = () => {
    setEditState({ open: true, mode: "create" });
  };

  const handleEdit = (c: Connection) => {
    setEditState({ open: true, mode: "edit", initial: c });
  };

  const handleSubmit = (next: Connection) => {
    const existingIndex = connections.findIndex((c) => c.id === next.id);
    const nextConnections =
      existingIndex >= 0
        ? connections.map((c) => (c.id === next.id ? next : c))
        : [...connections, next];
    updateSettings({
      aiConfig: { ...aiConfig, connections: nextConnections },
    });
  };

  const handleRequestDelete = (c: Connection) => {
    const dependentProfiles = aiConfig.profiles.filter(
      (p) => p.connectionId === c.id,
    );
    const dependentProfileIds = dependentProfiles.map((p) => p.id);
    const dependentProfileNames = dependentProfiles.map((p) => p.name);

    const affectedAssignments: string[] = [];
    if (
      aiConfig.assignments.chatProfileId !== null &&
      dependentProfileIds.includes(aiConfig.assignments.chatProfileId)
    ) {
      affectedAssignments.push("Chat");
    }
    if (
      aiConfig.assignments.aiActionsProfileId !== null &&
      dependentProfileIds.includes(aiConfig.assignments.aiActionsProfileId)
    ) {
      affectedAssignments.push("AI actions");
    }

    const affectedSlotNames = dictation.slots
      .filter(
        (s) => s.profileId !== null && dependentProfileIds.includes(s.profileId),
      )
      .map((s) => s.name);

    setDeleteState({
      open: true,
      connection: c,
      dependentProfileIds,
      dependentProfileNames,
      affectedAssignments,
      affectedSlotNames,
    });
  };

  const handleConfirmDelete = async () => {
    if (!deleteState) return;
    const { connection: c, dependentProfileIds } = deleteState;

    const nextConfig: AIConfig = {
      connections: aiConfig.connections.filter((x) => x.id !== c.id),
      profiles: aiConfig.profiles.filter((p) => p.connectionId !== c.id),
      assignments: {
        chatProfileId:
          aiConfig.assignments.chatProfileId !== null &&
          dependentProfileIds.includes(aiConfig.assignments.chatProfileId)
            ? null
            : aiConfig.assignments.chatProfileId,
        aiActionsProfileId:
          aiConfig.assignments.aiActionsProfileId !== null &&
          dependentProfileIds.includes(aiConfig.assignments.aiActionsProfileId)
            ? null
            : aiConfig.assignments.aiActionsProfileId,
      },
    };

    const nextSlots = dictation.slots.map((s) =>
      s.profileId !== null && dependentProfileIds.includes(s.profileId)
        ? { ...s, profileId: null }
        : s,
    );

    // Clear per-chat overrides that pointed at deleted Profiles. We don't
    // have an index of which contexts had which profile, so we walk the
    // known assignment slots and clear any chat context settings whose
    // profile_id ended up dangling. resolveProfile already tolerates
    // dangling references at read time, but proactively clearing keeps
    // the table tidy.
    for (const pid of dependentProfileIds) {
      const current = await getChatContextProfileId(pid).catch(() => null);
      if (current === pid) {
        await clearChatContextProfile(pid).catch(() => {});
      }
    }

    updateSettings({
      aiConfig: nextConfig,
      dictation: { ...dictation, slots: nextSlots },
    });
    setDeleteState(null);
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <p className="text-xs text-muted-foreground">
          {connections.length === 0
            ? "Connect an AI provider to get started."
            : `${connections.length} connection${connections.length === 1 ? "" : "s"}`}
        </p>
        {connections.length > 0 && (
          <Button size="sm" variant="outline" onClick={handleCreate} className="text-xs">
            <Plus className="mr-1 h-3 w-3" />
            Add Connection
          </Button>
        )}
      </div>

      {connections.length === 0 ? (
        <EmptyState onAdd={handleCreate} />
      ) : (
        <div className="rounded-md border border-border bg-card divide-y divide-border">
          {connections.map((c) => (
            <ConnectionRow
              key={c.id}
              connection={c}
              onEdit={() => handleEdit(c)}
              onDelete={() => handleRequestDelete(c)}
            />
          ))}
        </div>
      )}

      <ConnectionEditorDialog
        open={editState.open}
        onOpenChange={(open) => setEditState((s) => ({ ...s, open }))}
        mode={editState.mode}
        initial={editState.initial}
        onSubmit={handleSubmit}
      />

      {deleteState && (
        <AlertDialog
          open={deleteState.open}
          onOpenChange={(open) =>
            setDeleteState(open ? deleteState : null)
          }
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                Delete connection &ldquo;{deleteState.connection.name}&rdquo;?
              </AlertDialogTitle>
              <AlertDialogDescription asChild>
                <div className="space-y-2 text-xs">
                  {deleteState.dependentProfileNames.length === 0 ? (
                    <p>No Profiles or features depend on this connection.</p>
                  ) : (
                    <>
                      <p>
                        This will also remove{" "}
                        {deleteState.dependentProfileNames.length} Profile
                        {deleteState.dependentProfileNames.length === 1 ? "" : "s"}
                        :
                      </p>
                      <ul className="list-disc pl-5 text-muted-foreground">
                        {deleteState.dependentProfileNames.map((n, i) => (
                          <li key={i}>{n}</li>
                        ))}
                      </ul>
                      {(deleteState.affectedAssignments.length > 0 ||
                        deleteState.affectedSlotNames.length > 0) && (
                        <p>
                          The following features will be unassigned and stop
                          using AI until reassigned:
                        </p>
                      )}
                      {(deleteState.affectedAssignments.length > 0 ||
                        deleteState.affectedSlotNames.length > 0) && (
                        <ul className="list-disc pl-5 text-muted-foreground">
                          {deleteState.affectedAssignments.map((a, i) => (
                            <li key={`a-${i}`}>{a}</li>
                          ))}
                          {deleteState.affectedSlotNames.map((s, i) => (
                            <li key={`s-${i}`}>Dictation slot &ldquo;{s}&rdquo;</li>
                          ))}
                        </ul>
                      )}
                    </>
                  )}
                  <p className="text-muted-foreground">This can&rsquo;t be undone.</p>
                </div>
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-white hover:bg-destructive/90"
                onClick={handleConfirmDelete}
              >
                Delete connection
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}
    </div>
  );
}

function ConnectionRow({
  connection,
  onEdit,
  onDelete,
}: {
  connection: Connection;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const Icon = KIND_ICON[connection.kind];
  const modelCount = connection.availableModels?.length;
  const modelStatus = useMemo(() => {
    if (connection.fetchError) return "Could not fetch models";
    if (modelCount === undefined) return "Models not fetched";
    if (modelCount === 0) return "No models reported";
    return `${modelCount} model${modelCount === 1 ? "" : "s"}`;
  }, [connection.fetchError, modelCount]);

  return (
    <div className="flex items-center gap-3 px-3 py-2.5">
      <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{connection.name}</span>
          <Badge variant="secondary" className="text-[10px] px-1.5 py-0">
            {KIND_LABEL[connection.kind]}
          </Badge>
        </div>
        <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
          {connection.baseUrl}
        </div>
        <div className="mt-0.5 text-[10px] text-muted-foreground">
          {modelStatus}
        </div>
      </div>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground hover:text-foreground"
        onClick={onEdit}
        aria-label={`Edit ${connection.name}`}
      >
        <Pencil className="h-3.5 w-3.5" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground hover:text-destructive"
        onClick={onDelete}
        aria-label={`Delete ${connection.name}`}
      >
        <Trash2 className="h-3.5 w-3.5" />
      </Button>
    </div>
  );
}

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="rounded-md border border-dashed border-border bg-card px-6 py-8 text-center">
      <div className="mx-auto mb-3 flex h-9 w-9 items-center justify-center rounded-full bg-muted">
        <Server className="h-4 w-4 text-muted-foreground" />
      </div>
      <h3 className="text-sm font-medium">No connections yet</h3>
      <p className="mx-auto mt-1 max-w-xs text-xs text-muted-foreground leading-relaxed">
        Connect to OpenAI, OpenRouter, or any OpenAI-compatible server to
        start using AI features like dictation cleanup and chat.
      </p>
      <Button size="sm" onClick={onAdd} className="mt-4 text-xs">
        <Plus className="mr-1 h-3 w-3" />
        Add Connection
      </Button>
    </div>
  );
}
