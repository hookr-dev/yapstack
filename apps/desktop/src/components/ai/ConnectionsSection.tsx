import { useEffect, useState } from "react";
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
import {
  AlertTriangle,
  Loader2,
  Network,
  Pencil,
  Plus,
  RefreshCw,
  Server,
  Sparkles,
  Trash2,
} from "lucide-react";
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
import { useRefreshConnectionModels } from "@/hooks/useRefreshConnectionModels";

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
  affectedAssignments: string[];
  affectedSlotNames: string[];
}

export function ConnectionsSection({
  autoOpenEditor = false,
  onAutoOpenConsumed,
}: {
  autoOpenEditor?: boolean;
  onAutoOpenConsumed?: () => void;
} = {}) {
  const aiConfig = useAppStore((s) => s.settings.aiConfig);
  const dictation = useAppStore((s) => s.settings.dictation);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const { refresh, refreshingId } = useRefreshConnectionModels();

  const [editState, setEditState] = useState<EditState>({
    open: false,
    mode: "create",
  });
  const [deleteState, setDeleteState] = useState<DeleteState | null>(null);

  useEffect(() => {
    if (autoOpenEditor) {
      setEditState({ open: true, mode: "create" });
      onAutoOpenConsumed?.();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const connections = aiConfig.connections;
  const hasConnections = connections.length > 0;

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
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h4 className="text-[11px] font-medium uppercase text-muted-foreground">
            Connections
          </h4>
          <p className="text-[11px] text-muted-foreground/70">
            A provider&rsquo;s API key and endpoint.
          </p>
        </div>
        {hasConnections && (
          <Button
            size="sm"
            variant="outline"
            onClick={handleCreate}
            className="shrink-0 text-xs"
          >
            <Plus className="mr-1 h-3 w-3" />
            Add Connection
          </Button>
        )}
      </div>

      {hasConnections ? (
        <div className="divide-y divide-border rounded-md border border-border bg-card">
          {connections.map((c) => (
            <ConnectionRow
              key={c.id}
              connection={c}
              refreshing={refreshingId === c.id}
              onRefresh={() => refresh(c.id)}
              onEdit={() => handleEdit(c)}
              onDelete={() => handleRequestDelete(c)}
            />
          ))}
        </div>
      ) : (
        <EmptyState onAdd={handleCreate} />
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
  refreshing,
  onRefresh,
  onEdit,
  onDelete,
}: {
  connection: Connection;
  refreshing: boolean;
  onRefresh: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const Icon = KIND_ICON[connection.kind];

  return (
    <div className="flex items-center gap-2.5 px-3 py-2">
      <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-xs font-medium">{connection.name}</span>
          <Badge variant="secondary" className="px-1.5 py-0 text-[10px]">
            {KIND_LABEL[connection.kind]}
          </Badge>
        </div>
        <SecondaryLine connection={connection} refreshing={refreshing} />
      </div>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground hover:text-foreground"
        onClick={onRefresh}
        disabled={refreshing}
        aria-label={`Refresh models for ${connection.name}`}
        title="Refresh models"
      >
        {refreshing ? (
          <Loader2 className="h-3 w-3 animate-spin" />
        ) : (
          <RefreshCw className="h-3 w-3" />
        )}
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground hover:text-foreground"
        onClick={onEdit}
        aria-label={`Edit ${connection.name}`}
      >
        <Pencil className="h-3 w-3" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground hover:text-destructive"
        onClick={onDelete}
        aria-label={`Delete ${connection.name}`}
      >
        <Trash2 className="h-3 w-3" />
      </Button>
    </div>
  );
}

function SecondaryLine({
  connection,
  refreshing,
}: {
  connection: Connection;
  refreshing: boolean;
}) {
  return (
    <div className="mt-0.5 flex min-w-0 items-center gap-1 text-[11px] text-muted-foreground">
      <span className="truncate">{connection.baseUrl}</span>
      <span className="shrink-0 text-muted-foreground/50">·</span>
      <StatusInline connection={connection} refreshing={refreshing} />
    </div>
  );
}

function StatusInline({
  connection,
  refreshing,
}: {
  connection: Connection;
  refreshing: boolean;
}) {
  if (refreshing) {
    return (
      <span className="flex shrink-0 items-center gap-1">
        <Loader2 className="h-2.5 w-2.5 animate-spin" />
        Fetching…
      </span>
    );
  }
  if (connection.fetchError) {
    return (
      <span
        className="flex shrink-0 items-center gap-1 text-destructive"
        title={connection.fetchError}
      >
        <AlertTriangle className="h-2.5 w-2.5" />
        Fetch failed
      </span>
    );
  }
  const count = connection.availableModels?.length;
  if (count === undefined) {
    return <span className="shrink-0">not fetched</span>;
  }
  if (count === 0) {
    return <span className="shrink-0">no models</span>;
  }
  return (
    <span className="shrink-0">
      {count} model{count === 1 ? "" : "s"}
    </span>
  );
}

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="rounded-md border border-dashed border-border px-6 py-5 text-center">
      <div className="mx-auto mb-2 flex h-7 w-7 items-center justify-center rounded-full bg-muted">
        <Server className="h-3 w-3 text-muted-foreground" />
      </div>
      <p className="mx-auto max-w-xs text-[11px] leading-relaxed text-muted-foreground">
        Connect to OpenAI, OpenRouter, or any OpenAI-compatible server to
        start using AI features like dictation cleanup and chat.
      </p>
      <Button size="sm" onClick={onAdd} className="mt-3 text-xs">
        <Plus className="mr-1 h-3 w-3" />
        Add Connection
      </Button>
    </div>
  );
}
