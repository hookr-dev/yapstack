import { useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { Button } from "@/components/ui/button";
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
import { ArrowRight, Pencil, Plus, Sliders, Trash2 } from "lucide-react";
import type { AIConfig, Profile } from "@/lib/ai";
import { ProfileEditorDialog } from "./ProfileEditorDialog";
import { AssignmentsSummary } from "./AssignmentsSummary";
import { clearChatContextProfile } from "@/lib/db";

interface EditState {
  open: boolean;
  mode: "create" | "edit";
  initial?: Profile;
}

interface DeleteState {
  open: boolean;
  profile: Profile;
  affectedAssignments: string[];
  affectedSlotNames: string[];
}

export function ProfilesSubTab({
  onGoToConnections,
}: {
  onGoToConnections: () => void;
}) {
  const aiConfig = useAppStore((s) => s.settings.aiConfig);
  const dictation = useAppStore((s) => s.settings.dictation);
  const updateSettings = useAppStore((s) => s.updateSettings);

  const [editState, setEditState] = useState<EditState>({
    open: false,
    mode: "create",
  });
  const [deleteState, setDeleteState] = useState<DeleteState | null>(null);

  const { profiles, connections } = aiConfig;

  const handleCreate = () => {
    setEditState({ open: true, mode: "create" });
  };

  const handleEdit = (p: Profile) => {
    setEditState({ open: true, mode: "edit", initial: p });
  };

  const handleSubmit = (next: Profile) => {
    const existingIndex = profiles.findIndex((p) => p.id === next.id);
    const nextProfiles =
      existingIndex >= 0
        ? profiles.map((p) => (p.id === next.id ? next : p))
        : [...profiles, next];
    updateSettings({ aiConfig: { ...aiConfig, profiles: nextProfiles } });
  };

  const handleRequestDelete = (p: Profile) => {
    const affectedAssignments: string[] = [];
    if (aiConfig.assignments.chatProfileId === p.id) {
      affectedAssignments.push("Chat");
    }
    if (aiConfig.assignments.aiActionsProfileId === p.id) {
      affectedAssignments.push("AI actions");
    }
    const affectedSlotNames = dictation.slots
      .filter((s) => s.profileId === p.id)
      .map((s) => s.name);

    setDeleteState({
      open: true,
      profile: p,
      affectedAssignments,
      affectedSlotNames,
    });
  };

  const handleConfirmDelete = async () => {
    if (!deleteState) return;
    const { profile: p } = deleteState;

    const nextConfig: AIConfig = {
      connections: aiConfig.connections,
      profiles: aiConfig.profiles.filter((x) => x.id !== p.id),
      assignments: {
        chatProfileId:
          aiConfig.assignments.chatProfileId === p.id
            ? null
            : aiConfig.assignments.chatProfileId,
        aiActionsProfileId:
          aiConfig.assignments.aiActionsProfileId === p.id
            ? null
            : aiConfig.assignments.aiActionsProfileId,
      },
    };

    const nextSlots = dictation.slots.map((s) =>
      s.profileId === p.id ? { ...s, profileId: null } : s,
    );

    await clearChatContextProfile(p.id).catch(() => {});

    updateSettings({
      aiConfig: nextConfig,
      dictation: { ...dictation, slots: nextSlots },
    });
    setDeleteState(null);
  };

  const hasConnections = connections.length > 0;
  const hasProfiles = profiles.length > 0;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <p className="text-xs text-muted-foreground">
          {hasProfiles
            ? `${profiles.length} profile${profiles.length === 1 ? "" : "s"}`
            : "Profiles bundle a Connection and a model."}
        </p>
        {hasProfiles && (
          <Button
            size="sm"
            variant="outline"
            onClick={handleCreate}
            disabled={!hasConnections}
            className="text-xs"
          >
            <Plus className="mr-1 h-3 w-3" />
            Add Profile
          </Button>
        )}
      </div>

      {!hasProfiles ? (
        <EmptyState
          hasConnections={hasConnections}
          onAdd={handleCreate}
          onGoToConnections={onGoToConnections}
        />
      ) : (
        <div className="rounded-md border border-border bg-card divide-y divide-border">
          {profiles.map((p) => {
            const conn = connections.find((c) => c.id === p.connectionId);
            return (
              <ProfileRow
                key={p.id}
                profile={p}
                connectionName={conn?.name ?? null}
                onEdit={() => handleEdit(p)}
                onDelete={() => handleRequestDelete(p)}
              />
            );
          })}
        </div>
      )}

      <AssignmentsSummary />

      <ProfileEditorDialog
        open={editState.open}
        onOpenChange={(open) => setEditState((s) => ({ ...s, open }))}
        mode={editState.mode}
        connections={connections}
        initial={editState.initial}
        onSubmit={handleSubmit}
      />

      {deleteState && (
        <AlertDialog
          open={deleteState.open}
          onOpenChange={(open) => setDeleteState(open ? deleteState : null)}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                Delete profile &ldquo;{deleteState.profile.name}&rdquo;?
              </AlertDialogTitle>
              <AlertDialogDescription asChild>
                <div className="space-y-2 text-xs">
                  {deleteState.affectedAssignments.length === 0 &&
                  deleteState.affectedSlotNames.length === 0 ? (
                    <p>No features depend on this Profile.</p>
                  ) : (
                    <>
                      <p>
                        The following features will be unassigned and stop
                        using AI until reassigned:
                      </p>
                      <ul className="list-disc pl-5 text-muted-foreground">
                        {deleteState.affectedAssignments.map((a, i) => (
                          <li key={`a-${i}`}>{a}</li>
                        ))}
                        {deleteState.affectedSlotNames.map((s, i) => (
                          <li key={`s-${i}`}>Dictation slot &ldquo;{s}&rdquo;</li>
                        ))}
                      </ul>
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
                Delete profile
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}
    </div>
  );
}

function ProfileRow({
  profile,
  connectionName,
  onEdit,
  onDelete,
}: {
  profile: Profile;
  connectionName: string | null;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <div className="flex items-center gap-3 px-3 py-2.5">
      <Sliders className="h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium">{profile.name}</div>
        <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
          {connectionName ?? (
            <span className="text-destructive">
              Original connection deleted
            </span>
          )}
          {" · "}
          {profile.model}
        </div>
      </div>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground hover:text-foreground"
        onClick={onEdit}
        aria-label={`Edit ${profile.name}`}
      >
        <Pencil className="h-3.5 w-3.5" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground hover:text-destructive"
        onClick={onDelete}
        aria-label={`Delete ${profile.name}`}
      >
        <Trash2 className="h-3.5 w-3.5" />
      </Button>
    </div>
  );
}

function EmptyState({
  hasConnections,
  onAdd,
  onGoToConnections,
}: {
  hasConnections: boolean;
  onAdd: () => void;
  onGoToConnections: () => void;
}) {
  if (!hasConnections) {
    return (
      <div className="rounded-md border border-dashed border-border bg-card px-6 py-8 text-center">
        <div className="mx-auto mb-3 flex h-9 w-9 items-center justify-center rounded-full bg-muted">
          <Sliders className="h-4 w-4 text-muted-foreground" />
        </div>
        <h3 className="text-sm font-medium">No profiles yet</h3>
        <p className="mx-auto mt-1 max-w-xs text-xs text-muted-foreground leading-relaxed">
          A Profile bundles a Connection and a model. Add a Connection
          first, then come back here to create one.
        </p>
        <Button size="sm" onClick={onGoToConnections} className="mt-4 text-xs">
          Go to Connections
          <ArrowRight className="ml-1 h-3 w-3" />
        </Button>
      </div>
    );
  }
  return (
    <div className="rounded-md border border-dashed border-border bg-card px-6 py-8 text-center">
      <div className="mx-auto mb-3 flex h-9 w-9 items-center justify-center rounded-full bg-muted">
        <Sliders className="h-4 w-4 text-muted-foreground" />
      </div>
      <h3 className="text-sm font-medium">No profiles yet</h3>
      <p className="mx-auto mt-1 max-w-xs text-xs text-muted-foreground leading-relaxed">
        Profiles bundle a Connection and a model so you can assign different
        models to Chat, dictation, and AI actions.
      </p>
      <Button size="sm" onClick={onAdd} className="mt-4 text-xs">
        <Plus className="mr-1 h-3 w-3" />
        Add Profile
      </Button>
    </div>
  );
}
