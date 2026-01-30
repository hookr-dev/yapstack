import { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { Ban } from "lucide-react";
import { cn } from "@/lib/utils";
import { ICON_OPTIONS, COLOR_OPTIONS } from "@/lib/folder-constants";

export interface FolderDialogData {
  name: string;
  icon: string | null;
  color: string | null;
  description: string | null;
}

interface FolderDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mode: "create" | "edit";
  initialData?: Partial<FolderDialogData>;
  parentId?: string;
  parentName?: string;
  onSubmit: (data: FolderDialogData) => void;
}

export function FolderDialog({
  open,
  onOpenChange,
  mode,
  initialData,
  parentName,
  onSubmit,
}: FolderDialogProps) {
  const [name, setName] = useState("");
  const [icon, setIcon] = useState<string | null>(null);
  const [color, setColor] = useState<string | null>(null);
  const [description, setDescription] = useState("");

  const initName = initialData?.name;
  const initIcon = initialData?.icon;
  const initColor = initialData?.color;
  const initDescription = initialData?.description;

  useEffect(() => {
    if (open) {
      setName(initName ?? "");
      setIcon(initIcon ?? null);
      setColor(initColor ?? null);
      setDescription(initDescription ?? "");
    }
  }, [open, initName, initIcon, initColor, initDescription]);

  const handleSubmit = () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    onSubmit({
      name: trimmed,
      icon,
      color,
      description: description.trim() || null,
    });
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {mode === "create" ? "New Folder" : "Edit Folder"}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {/* Parent context */}
          {mode === "create" && parentName && (
            <p className="text-xs text-muted-foreground">
              Creating inside: <span className="font-medium text-foreground">{parentName}</span>
            </p>
          )}

          {/* Name */}
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-muted-foreground">
              Name
            </label>
            <Input
              placeholder="Folder name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
              autoFocus
            />
          </div>

          {/* Icon picker */}
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-muted-foreground">
              Icon
            </label>
            <div className="grid grid-cols-8 gap-1">
              <button
                type="button"
                className={cn(
                  "flex h-8 w-8 items-center justify-center rounded-md transition-all",
                  icon === null
                    ? "ring-2 ring-primary bg-accent"
                    : "hover:bg-muted",
                )}
                onClick={() => setIcon(null)}
              >
                <Ban className="h-3.5 w-3.5 text-muted-foreground" />
              </button>
              {ICON_OPTIONS.map((opt) => {
                const Icon = opt.icon;
                return (
                  <button
                    key={opt.name}
                    type="button"
                    className={cn(
                      "flex h-8 w-8 items-center justify-center rounded-md transition-all",
                      icon === opt.name
                        ? "ring-2 ring-primary bg-accent"
                        : "hover:bg-muted",
                    )}
                    onClick={() => setIcon(opt.name)}
                  >
                    <Icon className="h-4 w-4" />
                  </button>
                );
              })}
            </div>
          </div>

          {/* Color palette */}
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-muted-foreground">
              Color
            </label>
            <div className="flex items-center gap-1.5">
              {COLOR_OPTIONS.map((c, i) => (
                <button
                  key={i}
                  type="button"
                  className={cn(
                    "h-5 w-5 rounded-full border transition-all",
                    c === color
                      ? "scale-125 ring-2 ring-primary ring-offset-1 ring-offset-background"
                      : "hover:scale-110",
                    c === null && "border-muted-foreground/30",
                  )}
                  style={c ? { backgroundColor: c, borderColor: c } : undefined}
                  onClick={() => setColor(c)}
                >
                  {c === null && (
                    <Ban className="h-3 w-3 text-muted-foreground mx-auto" />
                  )}
                </button>
              ))}
            </div>
          </div>

          {/* Description */}
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-muted-foreground">
              Description{" "}
              <span className="text-muted-foreground/60">(optional)</span>
            </label>
            <Textarea
              placeholder="Add context for AI chat..."
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={3}
              className="resize-none text-xs"
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={!name.trim()}>
            {mode === "create" ? "Create" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
