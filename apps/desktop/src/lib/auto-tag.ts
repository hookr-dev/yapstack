import type { DbFolder } from "./db";

export interface FolderSuggestion {
  type: "folder";
  id: string;
  name: string;
  icon: string | null;
  color: string | null;
  matchCount: number;
}

const MIN_NAME_LENGTH = 4;
const MIN_MATCHES_TO_SUGGEST = 2;

export interface FolderKeywordEntry {
  id: string;
  name: string;
  icon: string | null;
  color: string | null;
}

export function buildFolderKeywordMap(
  folders: DbFolder[],
): Map<string, FolderKeywordEntry> {
  const map = new Map<string, FolderKeywordEntry>();
  for (const folder of folders) {
    if (folder.name.length >= MIN_NAME_LENGTH) {
      map.set(folder.name.toLowerCase(), {
        id: folder.id,
        name: folder.name,
        icon: folder.icon,
        color: folder.color,
      });
    }
  }
  return map;
}

function escapeRegex(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function scanTextForFolders(
  text: string,
  keywordMap: Map<string, FolderKeywordEntry>,
): string[] {
  const found: string[] = [];
  const lower = text.toLowerCase();
  for (const [keyword] of keywordMap) {
    const regex = new RegExp(`\\b${escapeRegex(keyword)}\\b`, "i");
    if (regex.test(lower)) {
      found.push(keyword);
    }
  }
  return found;
}

export class FolderSuggestionTracker {
  private matchCounts = new Map<string, number>();
  private dismissed = new Set<string>();
  private accepted = new Set<string>();
  private existingFolderIds: Set<string>;

  constructor(existingFolderIds: string[]) {
    this.existingFolderIds = new Set(existingFolderIds);
  }

  processSegment(
    text: string,
    keywordMap: Map<string, FolderKeywordEntry>,
  ): FolderSuggestion[] {
    const hits = scanTextForFolders(text, keywordMap);
    for (const keyword of hits) {
      this.matchCounts.set(keyword, (this.matchCounts.get(keyword) ?? 0) + 1);
    }

    const suggestions: FolderSuggestion[] = [];
    for (const [keyword, count] of this.matchCounts) {
      if (count < MIN_MATCHES_TO_SUGGEST) continue;
      if (this.dismissed.has(keyword) || this.accepted.has(keyword)) continue;

      const entry = keywordMap.get(keyword);
      if (!entry) continue;
      if (this.existingFolderIds.has(entry.id)) continue;

      suggestions.push({
        type: "folder",
        id: entry.id,
        name: entry.name,
        icon: entry.icon,
        color: entry.color,
        matchCount: count,
      });
    }

    return suggestions;
  }

  dismiss(keyword: string): void {
    this.dismissed.add(keyword.toLowerCase());
  }

  accept(keyword: string): void {
    this.accepted.add(keyword.toLowerCase());
  }

  addExistingFolder(folderId: string): void {
    this.existingFolderIds.add(folderId);
  }
}
