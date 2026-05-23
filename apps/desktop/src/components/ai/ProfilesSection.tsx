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
import { Pencil, Plus, Sliders, Trash2 } from "lucide-react";
import type { AIConfig, Profile } from "@/lib/ai";
import { ProfileEditorDialog } from "./ProfileEditorDialog";
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

export function ProfilesSection() {
  const aiConfig = useAppStore((s) => s.settings.aiConfig);
  const dictation = useAppStore((s) => s.settings.dictation);
  const updateSettings = useAppStore((s) => s.updateSettings);

  const [editState, setEditState] = useState<EditState>({
    open: false,
    mode: "create",
  });
  const [deleteState, setDeleteState] = useState<DeleteState | null>(null);

  const { profiles, connections } = aiConfig;
  const hasConnections = connections.length > 0;
  const hasProfiles = profiles.length > 0;

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

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <h4 className="text-[11px] font-medium uppercase text-muted-foreground">
          Profiles
        </h4>
        {hasProfiles && (
          <Button
            size="sm"
            variant="outline"
            onClick={handleCreate}
            disabled={!hasConnections}
            className="shrink-0 text-xs"
          >
            <Plus className="mr-1 h-3 w-3" />
            Add Profile
          </Button>
        )}
      </div>

      {hasProfiles ? (
        <div className="divide-y divide-border rounded-md border border-border bg-card">
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
      ) : (
        <EmptyState hasConnections={hasConnections} onAdd={handleCreate} />
      )}

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
    <div className="flex items-center gap-2.5 px-3 py-2">
      <Sliders className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-xs font-medium">{profile.name}</div>
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
        <Pencil className="h-3 w-3" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground hover:text-destructive"
        onClick={onDelete}
        aria-label={`Delete ${profile.name}`}
      >
        <Trash2 className="h-3 w-3" />
      </Button>
    </div>
  );
}

function EmptyState({
  hasConnections,
  onAdd,
}: {
  hasConnections: boolean;
  onAdd: () => void;
}) {
  return (
    <div className="rounded-md border border-dashed border-border px-6 py-5 text-center">
      <div className="mx-auto mb-2 flex h-7 w-7 items-center justify-center rounded-full bg-muted">
        <Sliders className="h-3 w-3 text-muted-foreground" />
      </div>
      <p className="mx-auto max-w-xs text-[11px] leading-relaxed text-muted-foreground">
        {hasConnections
          ? "Pair one of your Connections with a model so you can assign different models to different features."
          : "Add a Connection above first, then come back here to create a Profile."}
      </p>
      <Button
        size="sm"
        onClick={onAdd}
        disabled={!hasConnections}
        className="mt-3 text-xs"
      >
        <Plus className="mr-1 h-3 w-3" />
        Add Profile
      </Button>
    </div>
  );
}
