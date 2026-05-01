import { Check, ChevronDown, Folder, Sparkles, X } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Badge } from "@/components/ui/badge";
import type { FolderSuggestion } from "@/lib/auto-tag";
import type { DbFolder } from "@/lib/db";
import type { FolderTreeNode } from "@/lib/folder-tree";
import { ICON_MAP } from "@/lib/folder-constants";
import { folderBadgeStyle } from "@/lib/folder-badge";
import { cn } from "@/lib/utils";

interface AutoTagSuggestionsProps {
  suggestions: FolderSuggestion[];
  folders: DbFolder[];
  folderTree: FolderTreeNode[];
  onAccept: (suggestion: FolderSuggestion) => void;
  onApplyOverride: (folderId: string) => void;
  onDismiss: (suggestion: FolderSuggestion) => void;
}

export function AutoTagSuggestions({
  suggestions,
  folders,
  folderTree,
  onAccept,
  onApplyOverride,
  onDismiss,
}: AutoTagSuggestionsProps) {
  if (suggestions.length === 0) return null;

  return (
    <div className="flex flex-wrap items-center gap-2 px-4 py-2 border-b bg-muted/30 animate-in fade-in-0 slide-in-from-top-1 duration-150">
      <Badge variant="secondary" className="gap-1 text-[10px] uppercase tracking-wide font-medium">
        <Sparkles className="h-3 w-3" />
        Recommended
      </Badge>
      {suggestions.map((s) => (
        <SuggestionRow
          key={s.id}
          suggestion={s}
          folders={folders}
          folderTree={folderTree}
          onAccept={onAccept}
          onApplyOverride={onApplyOverride}
          onDismiss={onDismiss}
        />
      ))}
    </div>
  );
}

function SuggestionRow({
  suggestion,
  folders,
  folderTree,
  onAccept,
  onApplyOverride,
  onDismiss,
}: {
  suggestion: FolderSuggestion;
  folders: DbFolder[];
  folderTree: FolderTreeNode[];
  onAccept: (s: FolderSuggestion) => void;
  onApplyOverride: (folderId: string) => void;
  onDismiss: (s: FolderSuggestion) => void;
}) {
  // Only show the override picker when there's a meaningful alternative.
  // Single-folder users go through the dropdown for nothing — degrade to
  // inline check + dismiss buttons instead.
  const hasOverrideOptions = folders.length > 1;

  if (!hasOverrideOptions) {
    return (
      <div className="flex items-center gap-1">
        <SuggestionPill suggestion={suggestion} interactive={false} />
        <button
          onClick={() => onAccept(suggestion)}
          className="rounded-full p-0.5 text-muted-foreground hover:bg-primary/15 hover:text-primary transition-colors"
          aria-label={`Accept ${suggestion.name}`}
        >
          <Check className="h-3.5 w-3.5" />
        </button>
        <DismissButton onClick={() => onDismiss(suggestion)} label={suggestion.name} />
      </div>
    );
  }

  return (
    <div className="flex items-center gap-1">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            className="cursor-pointer outline-none"
            aria-label={`Confirm or change folder for ${suggestion.name}`}
          >
            <SuggestionPill suggestion={suggestion} interactive />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-[200px]">
          <DropdownMenuItem
            onClick={() => onAccept(suggestion)}
            className="bg-accent/40 focus:bg-accent"
          >
            <Check className="text-primary" />
            <FolderRow
              name={suggestion.name}
              icon={suggestion.icon}
              color={suggestion.color}
            />
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <div className="px-2 pt-1 pb-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
            Or pick another
          </div>
          {folderTree.map((node) => (
            <FolderPickerNode
              key={node.folder.id}
              node={node}
              skipFolderId={suggestion.id}
              onPick={onApplyOverride}
            />
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => onDismiss(suggestion)}>
            <X />
            <span>Dismiss suggestion</span>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <DismissButton onClick={() => onDismiss(suggestion)} label={suggestion.name} />
    </div>
  );
}

function SuggestionPill({
  suggestion,
  interactive,
}: {
  suggestion: FolderSuggestion;
  interactive: boolean;
}) {
  const FolderIcon = suggestion.icon ? ICON_MAP[suggestion.icon] : Folder;
  return (
    <div
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium transition-colors",
        suggestion.color ? "" : "bg-muted text-muted-foreground",
        interactive && "hover:brightness-95 dark:hover:brightness-110",
      )}
      style={folderBadgeStyle(suggestion.color)}
    >
      <FolderIcon className="h-2.5 w-2.5 shrink-0" />
      <span>{suggestion.name}</span>
      {interactive && <ChevronDown className="h-3 w-3 shrink-0 opacity-60" />}
    </div>
  );
}

function FolderRow({
  name,
  icon,
  color,
  trailing,
}: {
  name: string;
  icon: string | null;
  color: string | null;
  trailing?: React.ReactNode;
}) {
  const FolderIcon = icon ? ICON_MAP[icon] : Folder;
  return (
    <>
      <FolderIcon style={color ? { color } : undefined} />
      <span className="flex-1">{name}</span>
      {trailing}
    </>
  );
}

function FolderPickerNode({
  node,
  skipFolderId,
  onPick,
}: {
  node: FolderTreeNode;
  skipFolderId: string;
  onPick: (folderId: string) => void;
}) {
  const { folder, children } = node;
  const isRecommended = folder.id === skipFolderId;

  if (children.length === 0) {
    if (isRecommended) return null;
    return (
      <DropdownMenuItem onClick={() => onPick(folder.id)}>
        <FolderRow name={folder.name} icon={folder.icon} color={folder.color} />
      </DropdownMenuItem>
    );
  }

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        <FolderRow name={folder.name} icon={folder.icon} color={folder.color} />
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent>
        {!isRecommended && (
          <>
            <DropdownMenuItem onClick={() => onPick(folder.id)}>
              <FolderRow name={folder.name} icon={folder.icon} color={folder.color} />
            </DropdownMenuItem>
            <DropdownMenuSeparator />
          </>
        )}
        {children.map((child) => (
          <FolderPickerNode
            key={child.folder.id}
            node={child}
            skipFolderId={skipFolderId}
            onPick={onPick}
          />
        ))}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}

function DismissButton({ onClick, label }: { onClick: () => void; label: string }) {
  return (
    <button
      onClick={onClick}
      className="rounded-full p-0.5 text-muted-foreground/70 hover:bg-muted hover:text-foreground transition-colors"
      aria-label={`Dismiss suggestion for ${label}`}
    >
      <X className="h-3.5 w-3.5" />
    </button>
  );
}
