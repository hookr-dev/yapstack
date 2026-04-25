/**
 * Eval case format for the Chat agent loop. Each case seeds an in-memory
 * DB fixture, runs a sequence of Tool executions, and asserts on the
 * Tool observations + DB mutations. The runner is a regression net, not
 * a numerical relevance benchmark — it ensures Tool selection logic and
 * retrieval candidate lists do not silently change shape.
 */
import type {
  DbSession,
  DbFolder,
  DbSessionFolder,
  DbSegment,
  DbTag,
  DbSessionTag,
} from "@/lib/db";

export interface EvalFixture {
  sessions: DbSession[];
  folders: DbFolder[];
  sessionFolders: DbSessionFolder[];
  /** sessionId -> note HTML (omit for sessions with no note) */
  notes: Record<string, string>;
  /** sessionId -> segments in order */
  segments: Record<string, DbSegment[]>;
  tags: DbTag[];
  sessionTags: DbSessionTag[];
}

export interface EvalStep {
  tool: string;
  args: Record<string, unknown>;
  /** Override the default sessionId used in ToolContext (defaults to first session). */
  ctxSessionId?: string;
  expect: {
    /** Substring of the observation summary or evidence (whichever is truthy). */
    observationContains?: string;
    /** All of these must be present in observation.affectedIds. */
    affectedIdsIncludes?: string[];
    /** None of these may be present in observation.affectedIds. */
    affectedIdsExcludes?: string[];
    /** True when the result triggers the Undo window (kind=mutate AND undoData defined). */
    mutated?: boolean;
    /** Set when the Tool is expected to throw. */
    throws?: boolean;
  };
}

export interface EvalCase {
  name: string;
  description?: string;
  fixture: EvalFixture;
  steps: EvalStep[];
}
