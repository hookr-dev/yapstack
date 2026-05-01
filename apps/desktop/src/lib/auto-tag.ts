import type { DbFolder, DbTag } from "./db";

export interface FolderSuggestion {
  type: "folder";
  id: string;
  name: string;
  icon: string | null;
  color: string | null;
  /** Probability across all candidate folders, 0..1. */
  confidence: number;
  confidenceLevel: "high" | "medium";
  /** Sum of weighted keyword hits, length-normalized. Telemetry / debug. */
  score: number;
}

interface FolderProfile {
  id: string;
  name: string;
  icon: string | null;
  color: string | null;
  /** Lowercased keyword token → weight contribution to this folder. */
  keywords: Map<string, number>;
}

const FOLDER_NAME_WEIGHT = 3.0;
const FOLDER_DESCRIPTION_WEIGHT = 2.0;
const MIN_FOLDER_TOKEN_LEN = 4;
const MIN_TAG_TOKEN_LEN = 3;
const DEFAULT_PROB_THRESHOLD = 0.55;
const DEFAULT_MIN_SCORE = 1.0;
/** Words seen below this floor still divide by 1; above it we damp by sqrt. */
const LENGTH_NORM_FLOOR_WORDS = 100;

/**
 * Common English stopwords. Only excludes words ≥4 chars (shorter tokens are
 * already filtered by MIN_FOLDER_TOKEN_LEN / MIN_TAG_TOKEN_LEN). Without this
 * list a folder description like "Notes about the Q3 launch" would score
 * every transcript that contains "about" or "notes".
 */
const STOPWORDS = new Set([
  "about", "above", "after", "again", "against", "also", "another",
  "around", "because", "been", "before", "being", "below", "between",
  "both", "could", "does", "doing", "down", "during", "each", "even",
  "ever", "every", "from", "further", "have", "having", "here", "into",
  "itself", "just", "more", "most", "much", "must", "myself", "ourselves",
  "over", "same", "should", "some", "such", "than", "that", "their",
  "them", "themselves", "then", "there", "these", "they", "this", "those",
  "through", "under", "until", "very", "were", "what", "when", "where",
  "which", "while", "with", "would", "your", "yours", "yourself",
]);

function tokenize(s: string): string[] {
  return s
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
}

function isUsefulToken(tok: string, minLen: number): boolean {
  return tok.length >= minLen && !STOPWORDS.has(tok);
}

/**
 * Build per-folder keyword profiles. Each profile combines:
 *   - Folder-name tokens at a fixed high weight.
 *   - Tag-name tokens weighted by the tag's frequency within the folder (tf)
 *     and discounted by the number of folders that share the tag (idf-ish).
 *
 * The result is a static lookup the runtime tracker scores transcript text
 * against — rebuild only when folders/tags/session memberships change.
 */
export function buildFolderProfiles(
  folders: DbFolder[],
  tags: DbTag[],
  sessionFolderMap: Record<string, string[]>,
  sessionTagMap: Record<string, string[]>,
): FolderProfile[] {
  const tagsById = new Map(tags.map((t) => [t.id, t]));

  const folderSessionCount = new Map<string, number>();
  const folderTagCounts = new Map<string, Map<string, number>>();
  const tagFolderSet = new Map<string, Set<string>>();

  for (const [sessionId, folderIds] of Object.entries(sessionFolderMap)) {
    const tagIds = sessionTagMap[sessionId] ?? [];
    for (const fid of folderIds) {
      folderSessionCount.set(fid, (folderSessionCount.get(fid) ?? 0) + 1);
      let tagCounts = folderTagCounts.get(fid);
      if (!tagCounts) {
        tagCounts = new Map();
        folderTagCounts.set(fid, tagCounts);
      }
      for (const tid of tagIds) {
        tagCounts.set(tid, (tagCounts.get(tid) ?? 0) + 1);
        let folderSet = tagFolderSet.get(tid);
        if (!folderSet) {
          folderSet = new Set();
          tagFolderSet.set(tid, folderSet);
        }
        folderSet.add(fid);
      }
    }
  }

  const totalFolders = Math.max(1, folders.length);

  return folders.map((folder) => {
    const keywords = new Map<string, number>();

    for (const tok of tokenize(folder.name)) {
      if (!isUsefulToken(tok, MIN_FOLDER_TOKEN_LEN)) continue;
      keywords.set(tok, (keywords.get(tok) ?? 0) + FOLDER_NAME_WEIGHT);
    }

    if (folder.description) {
      for (const tok of tokenize(folder.description)) {
        if (!isUsefulToken(tok, MIN_FOLDER_TOKEN_LEN)) continue;
        keywords.set(tok, (keywords.get(tok) ?? 0) + FOLDER_DESCRIPTION_WEIGHT);
      }
    }

    const tagCounts = folderTagCounts.get(folder.id);
    const sessCount = folderSessionCount.get(folder.id) ?? 0;
    if (tagCounts && sessCount > 0) {
      for (const [tagId, count] of tagCounts) {
        const tag = tagsById.get(tagId);
        if (!tag) continue;
        const tf = count / sessCount;
        const folderSpread = tagFolderSet.get(tagId)?.size ?? 1;
        const idf = Math.log(1 + totalFolders / folderSpread);
        const weight = tf * idf;
        if (weight <= 0) continue;
        for (const tok of tokenize(tag.name)) {
          if (!isUsefulToken(tok, MIN_TAG_TOKEN_LEN)) continue;
          keywords.set(tok, (keywords.get(tok) ?? 0) + weight);
        }
      }
    }

    return {
      id: folder.id,
      name: folder.name,
      icon: folder.icon,
      color: folder.color,
      keywords,
    };
  });
}

/**
 * Scores incoming transcript segments against folder profiles and surfaces a
 * single recommended folder once it crosses both a probability threshold
 * (top folder vs. the rest) and an absolute minimum score (so a one-off
 * keyword hit on a long session doesn't trigger).
 */
export class FolderSuggestionTracker {
  private profiles: FolderProfile[];
  private scores = new Map<string, number>();
  private wordsSeen = 0;
  private dismissed = new Set<string>();
  private accepted = new Set<string>();
  private existingFolderIds: Set<string>;
  private threshold: number;
  private minScore: number;
  /**
   * Once the user picks any folder via the suggestion UI (accept the
   * recommendation or override with a different one), the channel is closed
   * for this session. Re-recommending after the user has already filed the
   * session is noisy. Dismiss is per-folder and does NOT flip this flag.
   */
  private completed = false;

  constructor(
    profiles: FolderProfile[],
    existingFolderIds: string[],
    options: { threshold?: number; minScore?: number } = {},
  ) {
    this.profiles = profiles;
    this.existingFolderIds = new Set(existingFolderIds);
    this.threshold = options.threshold ?? DEFAULT_PROB_THRESHOLD;
    this.minScore = options.minScore ?? DEFAULT_MIN_SCORE;
  }

  processSegment(text: string): FolderSuggestion[] {
    const tokens = tokenize(text);
    if (tokens.length === 0) return this.currentRecommendation();
    this.wordsSeen += tokens.length;

    const tokenCounts = new Map<string, number>();
    for (const tok of tokens) {
      tokenCounts.set(tok, (tokenCounts.get(tok) ?? 0) + 1);
    }

    for (const profile of this.profiles) {
      let segScore = 0;
      for (const [tok, count] of tokenCounts) {
        const w = profile.keywords.get(tok);
        if (w) segScore += w * count;
      }
      if (segScore > 0) {
        this.scores.set(profile.id, (this.scores.get(profile.id) ?? 0) + segScore);
      }
    }

    return this.currentRecommendation();
  }

  private currentRecommendation(): FolderSuggestion[] {
    if (this.completed) return [];
    if (this.scores.size === 0) return [];

    const lengthDamp = Math.max(1, Math.sqrt(this.wordsSeen / LENGTH_NORM_FLOOR_WORDS));

    let total = 0;
    let top: { profile: FolderProfile; score: number } | null = null;
    for (const profile of this.profiles) {
      if (this.dismissed.has(profile.id)) continue;
      if (this.accepted.has(profile.id)) continue;
      if (this.existingFolderIds.has(profile.id)) continue;
      const raw = this.scores.get(profile.id) ?? 0;
      const score = raw / lengthDamp;
      if (score <= 0) continue;
      total += score;
      if (!top || score > top.score) top = { profile, score };
    }

    if (!top || top.score < this.minScore) return [];

    const probability = total > 0 ? top.score / total : 0;
    if (probability < this.threshold) return [];

    return [
      {
        type: "folder",
        id: top.profile.id,
        name: top.profile.name,
        icon: top.profile.icon,
        color: top.profile.color,
        confidence: probability,
        confidenceLevel: probability >= 0.75 ? "high" : "medium",
        score: Math.round(top.score * 10) / 10,
      },
    ];
  }

  dismiss(folderId: string): void {
    this.dismissed.add(folderId);
  }

  accept(folderId: string): void {
    this.accepted.add(folderId);
    this.existingFolderIds.add(folderId);
    this.completed = true;
  }

  /** User picked a folder via the override path (not the recommended one). */
  complete(): void {
    this.completed = true;
  }

  addExistingFolder(folderId: string): void {
    this.existingFolderIds.add(folderId);
  }
}
