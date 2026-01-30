import { describe, it, expect } from "vitest";
import { buildFolderTree, getFolderPath, getRootFolder, isDescendantOf, getAncestorIds, getDescendantIds, findBranchConflicts, getDisplayFolders } from "./folder-tree";
import type { DbFolder } from "./db";

function makeFolder(overrides: Partial<DbFolder> & { id: string }): DbFolder {
  return {
    name: overrides.id,
    parent_id: null,
    sort_order: 0,
    icon: null,
    color: null,
    description: null,
    created_at: "2024-01-01",
    updated_at: "2024-01-01",
    ...overrides,
  };
}

describe("buildFolderTree", () => {
  it("returns empty array for empty input", () => {
    expect(buildFolderTree([])).toEqual([]);
  });

  it("returns a single root node", () => {
    const folders = [makeFolder({ id: "a" })];
    const tree = buildFolderTree(folders);
    expect(tree).toHaveLength(1);
    expect(tree[0].folder.id).toBe("a");
    expect(tree[0].children).toHaveLength(0);
  });

  it("returns two root nodes", () => {
    const folders = [makeFolder({ id: "a" }), makeFolder({ id: "b" })];
    const tree = buildFolderTree(folders);
    expect(tree).toHaveLength(2);
  });

  it("nests parent-child", () => {
    const folders = [
      makeFolder({ id: "parent" }),
      makeFolder({ id: "child", parent_id: "parent" }),
    ];
    const tree = buildFolderTree(folders);
    expect(tree).toHaveLength(1);
    expect(tree[0].children).toHaveLength(1);
    expect(tree[0].children[0].folder.id).toBe("child");
  });

  it("handles deep nesting", () => {
    const folders = [
      makeFolder({ id: "a" }),
      makeFolder({ id: "b", parent_id: "a" }),
      makeFolder({ id: "c", parent_id: "b" }),
    ];
    const tree = buildFolderTree(folders);
    expect(tree).toHaveLength(1);
    expect(tree[0].children[0].children[0].folder.id).toBe("c");
  });

  it("promotes orphan to root", () => {
    const folders = [makeFolder({ id: "orphan", parent_id: "nonexistent" })];
    const tree = buildFolderTree(folders);
    expect(tree).toHaveLength(1);
    expect(tree[0].folder.id).toBe("orphan");
  });
});

describe("getFolderPath", () => {
  const folders = [
    makeFolder({ id: "root" }),
    makeFolder({ id: "mid", parent_id: "root" }),
    makeFolder({ id: "leaf", parent_id: "mid" }),
  ];

  it("returns single element for root", () => {
    const path = getFolderPath(folders, "root");
    expect(path.map((f) => f.id)).toEqual(["root"]);
  });

  it("returns [root, child] for child", () => {
    const path = getFolderPath(folders, "mid");
    expect(path.map((f) => f.id)).toEqual(["root", "mid"]);
  });

  it("returns full chain for grandchild", () => {
    const path = getFolderPath(folders, "leaf");
    expect(path.map((f) => f.id)).toEqual(["root", "mid", "leaf"]);
  });

  it("returns empty for unknown ID", () => {
    expect(getFolderPath(folders, "nonexistent")).toEqual([]);
  });
});

describe("getRootFolder", () => {
  const folders = [
    makeFolder({ id: "root" }),
    makeFolder({ id: "child", parent_id: "root" }),
    makeFolder({ id: "grandchild", parent_id: "child" }),
  ];

  it("returns root when given a root folder ID", () => {
    const result = getRootFolder(folders, "root");
    expect(result?.id).toBe("root");
  });

  it("walks up parent_id chain for nested folders", () => {
    const result = getRootFolder(folders, "grandchild");
    expect(result?.id).toBe("root");
  });

  it("returns direct parent root for child", () => {
    const result = getRootFolder(folders, "child");
    expect(result?.id).toBe("root");
  });

  it("returns null for unknown folder ID", () => {
    expect(getRootFolder(folders, "nonexistent")).toBeNull();
  });

  it("handles orphaned parent_id gracefully", () => {
    const orphanFolders = [
      makeFolder({ id: "orphan", parent_id: "missing" }),
    ];
    const result = getRootFolder(orphanFolders, "orphan");
    expect(result?.id).toBe("orphan");
  });
});

describe("isDescendantOf", () => {
  const folders = [
    makeFolder({ id: "root" }),
    makeFolder({ id: "child", parent_id: "root" }),
    makeFolder({ id: "grandchild", parent_id: "child" }),
    makeFolder({ id: "sibling" }),
  ];

  it("returns true for direct child", () => {
    expect(isDescendantOf(folders, "child", "root")).toBe(true);
  });

  it("returns true for grandchild", () => {
    expect(isDescendantOf(folders, "grandchild", "root")).toBe(true);
  });

  it("returns false for sibling", () => {
    expect(isDescendantOf(folders, "sibling", "root")).toBe(false);
  });

  it("returns false for self", () => {
    expect(isDescendantOf(folders, "root", "root")).toBe(false);
  });

  it("returns false for unknown ID", () => {
    expect(isDescendantOf(folders, "nonexistent", "root")).toBe(false);
  });
});

describe("getAncestorIds", () => {
  const folders = [
    makeFolder({ id: "root" }),
    makeFolder({ id: "child", parent_id: "root" }),
    makeFolder({ id: "grandchild", parent_id: "child" }),
  ];

  it("returns empty for root folder", () => {
    expect(getAncestorIds(folders, "root")).toEqual([]);
  });

  it("returns [root] for child", () => {
    expect(getAncestorIds(folders, "child")).toEqual(["root"]);
  });

  it("returns [child, root] for grandchild", () => {
    expect(getAncestorIds(folders, "grandchild")).toEqual(["child", "root"]);
  });

  it("returns empty for unknown ID", () => {
    expect(getAncestorIds(folders, "nonexistent")).toEqual([]);
  });
});

describe("getDescendantIds", () => {
  const folders = [
    makeFolder({ id: "root" }),
    makeFolder({ id: "child", parent_id: "root" }),
    makeFolder({ id: "grandchild", parent_id: "child" }),
    makeFolder({ id: "sibling" }),
  ];

  it("returns all nested descendants for root", () => {
    const result = getDescendantIds(folders, "root");
    expect(result).toContain("child");
    expect(result).toContain("grandchild");
    expect(result).toHaveLength(2);
  });

  it("returns [grandchild] for child", () => {
    expect(getDescendantIds(folders, "child")).toEqual(["grandchild"]);
  });

  it("returns empty for leaf node", () => {
    expect(getDescendantIds(folders, "grandchild")).toEqual([]);
  });

  it("returns empty for unknown ID", () => {
    expect(getDescendantIds(folders, "nonexistent")).toEqual([]);
  });
});

describe("findBranchConflicts", () => {
  const folders = [
    makeFolder({ id: "A" }),
    makeFolder({ id: "A-sub", parent_id: "A" }),
    makeFolder({ id: "A-sub-sub", parent_id: "A-sub" }),
    makeFolder({ id: "B" }),
  ];

  it("detects parent conflict when adding to child", () => {
    // Session is in A, adding to A-sub → A is ancestor of A-sub
    expect(findBranchConflicts(folders, ["A"], "A-sub")).toEqual(["A"]);
  });

  it("detects child conflict when adding to parent", () => {
    // Session is in A-sub, adding to A → A-sub is descendant of A
    expect(findBranchConflicts(folders, ["A-sub"], "A")).toEqual(["A-sub"]);
  });

  it("detects deep conflicts", () => {
    // Session is in A and A-sub-sub, adding to A-sub
    const conflicts = findBranchConflicts(folders, ["A", "A-sub-sub"], "A-sub");
    expect(conflicts).toContain("A");
    expect(conflicts).toContain("A-sub-sub");
  });

  it("no conflict for sibling branches", () => {
    expect(findBranchConflicts(folders, ["B"], "A")).toEqual([]);
  });

  it("no conflict when session already in target", () => {
    expect(findBranchConflicts(folders, ["A"], "A")).toEqual([]);
  });
});

describe("getDisplayFolders", () => {
  const folders = [
    makeFolder({ id: "A", name: "FolderA", color: "#f00" }),
    makeFolder({ id: "A-sub", name: "Sub", parent_id: "A", color: "#0f0" }),
    makeFolder({ id: "A-sub-sub", name: "DeepSub", parent_id: "A-sub" }),
    makeFolder({ id: "B", name: "FolderB", color: "#00f" }),
  ];

  it("shows root folder in All view", () => {
    const result = getDisplayFolders(["A-sub"], folders, null);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("FolderA");
  });

  it("shows subfolder name when viewing parent folder", () => {
    const result = getDisplayFolders(["A-sub"], folders, "A");
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("Sub");
  });

  it("shows nothing when session is directly in context folder", () => {
    const result = getDisplayFolders(["A"], folders, "A");
    expect(result).toHaveLength(0);
  });

  it("shows multiple folders for multi-branch session in All view", () => {
    const result = getDisplayFolders(["A-sub", "B"], folders, null);
    expect(result).toHaveLength(2);
    expect(result.map(f => f.name)).toContain("FolderA");
    expect(result.map(f => f.name)).toContain("FolderB");
  });

  it("shows subfolder + other branch when viewing parent folder", () => {
    const result = getDisplayFolders(["A-sub", "B"], folders, "A");
    expect(result).toHaveLength(2);
    expect(result.map(f => f.name)).toContain("Sub");
    expect(result.map(f => f.name)).toContain("FolderB");
  });

  it("shows direct child of context for deep descendants", () => {
    const result = getDisplayFolders(["A-sub-sub"], folders, "A");
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("Sub");
  });

  it("returns empty for empty sessionFolderIds", () => {
    expect(getDisplayFolders([], folders, null)).toEqual([]);
  });
});
