import { useEffect, useMemo, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { Connection, Profile } from "@/lib/ai";

const SLOW_MODEL_PATTERN = /^(o1|o3|o4|chatgpt)/i;
const CUSTOM_MODEL_SENTINEL = "__custom_model__";

export interface ProfileEditorDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mode: "create" | "edit";
  connections: Connection[];
  initial?: Profile;
  onSubmit: (profile: Profile) => void;
}

export function ProfileEditorDialog({
  open,
  onOpenChange,
  mode,
  connections,
  initial,
  onSubmit,
}: ProfileEditorDialogProps) {
  const [name, setName] = useState("");
  const [connectionId, setConnectionId] = useState<string>("");
  const [model, setModel] = useState("");
  const [customModelMode, setCustomModelMode] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(initial?.name ?? "");
    setConnectionId(initial?.connectionId ?? connections[0]?.id ?? "");
    setModel(initial?.model ?? "");
    // If the persisted model isn't in the connection's catalog, start in
    // custom-text mode so the user can see + edit the raw value.
    const initConn = connections.find((c) => c.id === initial?.connectionId);
    const initInCatalog =
      initial?.model && initConn?.availableModels?.includes(initial.model);
    setCustomModelMode(
      !!initial?.model && !initInCatalog && !!initConn?.availableModels,
    );
  }, [open, initial, connections]);

  const selectedConnection = useMemo(
    () => connections.find((c) => c.id === connectionId) ?? null,
    [connections, connectionId],
  );

  const availableModels = selectedConnection?.availableModels;
  const hasCatalog = availableModels !== undefined && availableModels.length > 0;

  function handleConnectionChange(next: string) {
    setConnectionId(next);
    const nextConn = connections.find((c) => c.id === next);
    // Reset model + custom mode when switching connections — the previously
    // typed model very likely doesn't exist on the new server.
    if (nextConn?.availableModels && nextConn.availableModels.length > 0) {
      setModel(nextConn.availableModels[0]!);
      setCustomModelMode(false);
    } else {
      setModel("");
      setCustomModelMode(true);
    }
  }

  function handleSubmit() {
    if (!connectionId || !model.trim()) return;
    const conn = connections.find((c) => c.id === connectionId);
    const derivedName =
      name.trim() || `${conn?.name ?? "Profile"} · ${model.trim()}`;
    const id = initial?.id ?? crypto.randomUUID();
    onSubmit({
      id,
      name: derivedName,
      connectionId,
      model: model.trim(),
    });
    onOpenChange(false);
  }

  const canSubmit = !!connectionId && !!model.trim();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {mode === "create" ? "Add Profile" : "Edit Profile"}
          </DialogTitle>
          {mode === "create" && (
            <DialogDescription>
              A Profile bundles a Connection and a model. Assign it to Chat,
              AI actions, or any dictation slot.
            </DialogDescription>
          )}
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">
              Name
              <span className="ml-1 text-muted-foreground/60">(optional)</span>
            </Label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={
                selectedConnection
                  ? `${selectedConnection.name} · ${model || "model"}`
                  : "Profile name"
              }
              className="h-8 text-xs"
              autoFocus
            />
          </div>

          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">Connection</Label>
            <Select
              value={connectionId}
              onValueChange={handleConnectionChange}
              disabled={connections.length === 0}
            >
              <SelectTrigger className="h-8 w-full text-xs">
                <SelectValue placeholder="Choose a connection..." />
              </SelectTrigger>
              <SelectContent>
                {connections.map((c) => (
                  <SelectItem key={c.id} value={c.id} className="text-xs">
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">Model</Label>
            {hasCatalog && !customModelMode ? (
              <Select
                value={model || ""}
                onValueChange={(v) => {
                  if (v === CUSTOM_MODEL_SENTINEL) {
                    setCustomModelMode(true);
                  } else {
                    setModel(v);
                  }
                }}
              >
                <SelectTrigger className="h-8 w-full text-xs">
                  <SelectValue placeholder="Pick a model..." />
                </SelectTrigger>
                <SelectContent>
                  {availableModels!.map((m) => (
                    <SelectItem key={m} value={m} className="text-xs">
                      <span className="flex items-center gap-2">
                        {m}
                        {SLOW_MODEL_PATTERN.test(m) && (
                          <Badge variant="secondary" className="px-1 py-0 text-[9px]">
                            Slow
                          </Badge>
                        )}
                      </span>
                    </SelectItem>
                  ))}
                  <SelectItem value={CUSTOM_MODEL_SENTINEL} className="text-xs">
                    Custom model name…
                  </SelectItem>
                </SelectContent>
              </Select>
            ) : (
              <>
                <Input
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder="model-id"
                  className="h-8 text-xs"
                />
                {hasCatalog && (
                  <Button
                    variant="link"
                    size="inline"
                    onClick={() => setCustomModelMode(false)}
                  >
                    Pick from fetched models
                  </Button>
                )}
              </>
            )}
            {!selectedConnection && (
              <p className="text-[10px] text-muted-foreground">
                Add a Connection first to pick a model.
              </p>
            )}
            {selectedConnection && !hasCatalog && (
              <p className="text-[10px] text-muted-foreground leading-relaxed">
                No model catalog cached for this Connection. Type the model
                identifier the server expects (e.g. <code>llama3</code>,{" "}
                <code>qwen2.5</code>). Refresh from the Connection editor to
                fetch the catalog.
              </p>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button size="sm" onClick={handleSubmit} disabled={!canSubmit}>
            {mode === "create" ? "Add Profile" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
