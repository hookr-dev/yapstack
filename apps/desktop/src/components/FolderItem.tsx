import { useState, useMemo, memo } from "react";
import { useAppStore } from "@/stores/appStore";
import type { DbFolder } from "@/lib/db";
import { getDescendantIds, type FolderTreeNode } from "@/lib/folder-tree";
import { cn } from "@/lib/utils";
import { Folder, FolderOpen, FolderPlus, ChevronRight, ChevronDown, Pencil, Trash2 } from "lucide-react";
import { useSortable, SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { FolderDialog } from "@/components/FolderDialog";
import type { FolderDialogData } from "@/components/FolderDialog";
import { ICON_MAP } from "@/lib/folder-constants";

interface FolderItemProps {
  folder: DbFolder;
  childNodes: FolderTreeNode[];
  depth: number;
}

export const FolderItem = memo(function FolderItem({ folder, childNodes, depth }: FolderItemProps) {
  const sessionFolderMap = useAppStore((s) => s.sessionFolderMap);
  const listFilter = useAppStore((s) => s.listFilter);
  const setListFilter = useAppStore((s) => s.setListFilter);
  const navigateTo = useAppStore((s) => s.navigateTo);
  const updateFolder = useAppStore((s) => s.updateFolder);
  const deleteFolder = useAppStore((s) => s.deleteFolder);
  const createFolder = useAppStore((s) => s.createFolder);
  const folders = useAppStore((s) => s.folders);
  const folderChildMap = useAppStore((s) => s.folderChildMap);

  const [isOpen, setIsOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [subfolderOpen, setSubfolderOpen] = useState(false);

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
    isOver,
  } = useSortable({
    id: `folder-${folder.id}`,
    data: { type: "folder", folderId: folder.id, folderName: folder.name, parentId: folder.parent_id },
  });

  const isActive =
    listFilter.type === "folder" && listFilter.folderId === folder.id;
  const sessionCount = useMemo(() => {
    const targetIds = new Set([folder.id, ...getDescendantIds(folders, folder.id, folderChildMap)]);
    return Object.values(sessionFolderMap).filter((ids) =>
      ids.some((fId) => targetIds.has(fId)),
    ).length;
  }, [sessionFolderMap, folder.id, folders, folderChildMap]);

  const hasChildren = childNodes.length > 0;
  const isEmpty = !hasChildren && sessionCount === 0;

  const handleEdit = (data: FolderDialogData) => {
    updateFolder(folder.id, {
      name: data.name,
      icon: data.icon,
      color: data.color,
      description: data.description,
    });
  };

  const handleCreateSubfolder = (data: FolderDialogData) => {
    createFolder(data.name, folder.id, data.icon, data.color, data.description);
  };

  const iconColor = folder.color ?? undefined;
  const CustomIcon = folder.icon ? ICON_MAP[folder.icon] : null;

  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div
            ref={setNodeRef}
            {...listeners}
            {...attributes}
            style={{
              paddingLeft: depth * 12,
              transform: CSS.Transform.toString(transform),
              transition,
            }}
            className={isDragging ? "opacity-50" : undefined}
          >
            <Collapsible open={isOpen} onOpenChange={setIsOpen}>
              <CollapsibleTrigger asChild>
                <button
                  className={cn(
                    "flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-xs transition-colors",
                    isActive
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-muted/50 hover:text-foreground",
                    isOver && "border border-primary bg-primary/10",
                  )}
                  onClick={(e) => {
                    e.preventDefault();
                    setListFilter({ type: "folder", folderId: folder.id });
                    navigateTo("note-list");
                  }}
                >
                  {hasChildren && (
                    <span
                      className="-ml-0.5 shrink-0 p-0.5"
                      onClick={(e) => {
                        e.stopPropagation();
                        setIsOpen(!isOpen);
                      }}
                    >
                      {isOpen ? (
                        <ChevronDown className="h-3 w-3" />
                      ) : (
                        <ChevronRight className="h-3 w-3" />
                      )}
                    </span>
                  )}
                  {CustomIcon ? (
                    <CustomIcon
                      className="h-3.5 w-3.5 shrink-0"
                      style={iconColor ? { color: iconColor } : undefined}
                    />
                  ) : isOpen ? (
                    <FolderOpen
                      className="h-3.5 w-3.5 shrink-0"
                      style={iconColor ? { color: iconColor } : undefined}
                    />
                  ) : (
                    <Folder
                      className="h-3.5 w-3.5 shrink-0"
                      style={iconColor ? { color: iconColor } : undefined}
                    />
                  )}
                  <span className="truncate">{folder.name}</span>
                  {sessionCount > 0 && (
                    <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">
                      {sessionCount}
                    </span>
                  )}
                </button>
              </CollapsibleTrigger>
              <CollapsibleContent>
                <SortableContext items={childNodes.map(c => `folder-${c.folder.id}`)} strategy={verticalListSortingStrategy}>
                  {childNodes.map((child) => (
                    <FolderItem
                      key={child.folder.id}
                      folder={child.folder}
                      childNodes={child.children}
                      depth={depth + 1}
                    />
                  ))}
                </SortableContext>
                {isEmpty && (
                  <p
                    className="py-1 text-[11px] text-muted-foreground"
                    style={{ paddingLeft: (depth + 1) * 12 + 32 }}
                  >
                    Empty
                  </p>
                )}
              </CollapsibleContent>
            </Collapsible>
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuItem onClick={() => setSubfolderOpen(true)}>
            <FolderPlus />
            New Subfolder
          </ContextMenuItem>
          <ContextMenuItem onClick={() => setEditOpen(true)}>
            <Pencil />
            Edit
          </ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem
            className="text-destructive"
            onClick={() => deleteFolder(folder.id)}
          >
            <Trash2 />
            Delete
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>

      <FolderDialog
        open={editOpen}
        onOpenChange={setEditOpen}
        mode="edit"
        initialData={{
          name: folder.name,
          icon: folder.icon,
          color: folder.color,
          description: folder.description,
        }}
        onSubmit={handleEdit}
      />

      <FolderDialog
        open={subfolderOpen}
        onOpenChange={setSubfolderOpen}
        mode="create"
        parentId={folder.id}
        parentName={folder.name}
        onSubmit={handleCreateSubfolder}
      />
    </>
  );
});
