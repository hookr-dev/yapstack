import type { DbFolder } from "@/lib/db";

export interface FolderTreeNode {
  folder: DbFolder;
  children: FolderTreeNode[];
}

/** Build a nested tree from a flat folder list. Root nodes have parent_id === null. */
export function buildFolderTree(folders: DbFolder[]): FolderTreeNode[] {
  const map = new Map<string, FolderTreeNode>();
  for (const folder of folders) {
    map.set(folder.id, { folder, children: [] });
  }

  const roots: FolderTreeNode[] = [];
  for (const node of map.values()) {
    if (node.folder.parent_id && map.has(node.folder.parent_id)) {
      map.get(node.folder.parent_id)!.children.push(node);
    } else {
      roots.push(node);
    }
  }

  return roots;
}

/** Returns ancestor chain [root, ..., parent, current] for breadcrumb display. */
export function getFolderPath(folders: DbFolder[], folderId: string, byId?: Map<string, DbFolder>): DbFolder[] {
  const map = byId ?? new Map(folders.map((f) => [f.id, f]));
  const path: DbFolder[] = [];
  let current = map.get(folderId);
  while (current) {
    path.unshift(current);
    current = current.parent_id ? map.get(current.parent_id) : undefined;
  }
  return path;
}

/** Returns the root folder for a given folder ID by walking up the parent_id chain. */
export function getRootFolder(folders: DbFolder[], folderId: string, byId?: Map<string, DbFolder>): DbFolder | null {
  const map = byId ?? new Map(folders.map((f) => [f.id, f]));
  let current = map.get(folderId);
  if (!current) return null;
  while (current.parent_id) {
    const parent = map.get(current.parent_id);
    if (!parent) break;
    current = parent;
  }
  return current;
}

/** Returns true if `candidateDescendantId` is a descendant of `ancestorId`. */
export function isDescendantOf(
  folders: DbFolder[],
  candidateDescendantId: string,
  ancestorId: string,
  byId?: Map<string, DbFolder>,
): boolean {
  const map = byId ?? new Map(folders.map((f) => [f.id, f]));
  let current = map.get(candidateDescendantId);
  while (current) {
    if (current.parent_id === ancestorId) return true;
    current = current.parent_id ? map.get(current.parent_id) : undefined;
  }
  return false;
}

/** All ancestor folder IDs (exclusive of folderId), from parent up to root. */
export function getAncestorIds(folders: DbFolder[], folderId: string, byId?: Map<string, DbFolder>): string[] {
  const map = byId ?? new Map(folders.map((f) => [f.id, f]));
  const ancestors: string[] = [];
  let current = map.get(folderId);
  while (current?.parent_id) {
    const parent = map.get(current.parent_id);
    if (!parent) break;
    ancestors.push(parent.id);
    current = parent;
  }
  return ancestors;
}

/** Build a parent→children lookup map. */
export function buildChildMap(folders: DbFolder[]): Map<string, string[]> {
  const childMap = new Map<string, string[]>();
  for (const f of folders) {
    if (f.parent_id) {
      const children = childMap.get(f.parent_id) ?? [];
      children.push(f.id);
      childMap.set(f.parent_id, children);
    }
  }
  return childMap;
}

/** All descendant folder IDs (recursive children, exclusive of folderId). */
export function getDescendantIds(folders: DbFolder[], folderId: string, childMap?: Map<string, string[]>): string[] {
  const map = childMap ?? buildChildMap(folders);
  const descendants: string[] = [];
  const queue = [...(map.get(folderId) ?? [])];
  while (queue.length > 0) {
    const id = queue.shift()!;
    descendants.push(id);
    const children = map.get(id);
    if (children) queue.push(...children);
  }
  return descendants;
}

/** Folder IDs from sessionFolderIds that are ancestors or descendants of targetFolderId. */
export function findBranchConflicts(
  folders: DbFolder[],
  sessionFolderIds: string[],
  targetFolderId: string,
): string[] {
  const ancestors = new Set(getAncestorIds(folders, targetFolderId));
  const descendants = new Set(getDescendantIds(folders, targetFolderId));
  return sessionFolderIds.filter(
    (id) => id !== targetFolderId && (ancestors.has(id) || descendants.has(id)),
  );
}

export interface DisplayFolder {
  id: string;
  name: string;
  color: string | null;
  icon: string | null;
}

/**
 * Returns display-ready folder info for a session's folder assignments,
 * adapted to the current view context.
 */
export function getDisplayFolders(
  sessionFolderIds: string[],
  folders: DbFolder[],
  contextFolderId: string | null,
  byId?: Map<string, DbFolder>,
): DisplayFolder[] {
  if (sessionFolderIds.length === 0 || folders.length === 0) return [];

  const map = byId ?? new Map(folders.map((f) => [f.id, f]));
  const result: DisplayFolder[] = [];

  for (const fId of sessionFolderIds) {
    const folder = map.get(fId);
    if (!folder) continue;

    if (contextFolderId === null) {
      // All/Pinned view: show root folder of that branch
      const root = getRootFolder(folders, fId, map);
      if (root && !result.some((r) => r.id === root.id)) {
        result.push({ id: root.id, name: root.name, color: root.color, icon: root.icon });
      }
    } else if (fId === contextFolderId) {
      // Session is directly in the context folder — skip
      continue;
    } else if (isDescendantOf(folders, fId, contextFolderId, map)) {
      // Session is in a descendant of context → show the direct child of context in the path
      const path = getFolderPath(folders, fId, map);
      const contextIdx = path.findIndex((f) => f.id === contextFolderId);
      if (contextIdx >= 0 && contextIdx + 1 < path.length) {
        const directChild = path[contextIdx + 1];
        if (!result.some((r) => r.id === directChild.id)) {
          result.push({ id: directChild.id, name: directChild.name, color: directChild.color, icon: directChild.icon });
        }
      }
    } else {
      // Different branch — show root of that branch
      const root = getRootFolder(folders, fId, map);
      if (root && !result.some((r) => r.id === root.id)) {
        result.push({ id: root.id, name: root.name, color: root.color, icon: root.icon });
      }
    }
  }

  return result;
}
