import type { LucideIcon } from "lucide-react";
import { Sparkles, List, ListChecks, ClipboardList } from "lucide-react";
import type { SessionType } from "./db";

export interface ActionDefinition {
  id: string;
  label: string;
  description: string;
  icon: LucideIcon;
  requiresTranscript?: boolean;
  directive: string;
  /**
   * Verbose user-message content sent to the LLM when this action is
   * invoked from a button click. Defaults to `label` when omitted, but
   * a sentence-shaped prompt avoids ambiguity in a chat history
   * (clicking "Summarize" mid-conversation otherwise reads to the
   * model as a follow-up clarification rather than an action). The UI
   * still renders the action chip — the verbose text is hidden behind
   * `message.action`.
   */
  userPrompt?: string;
}

/**
 * Prepended to every action directive. Forbids clarification questions
 * and pins the source/destination so the model never asks "did you
 * mean the conversation or the notes?" when both are in context.
 */
const ACTION_PREAMBLE = `This is an action invocation, not a conversational request. Execute the action immediately using your tools. Do not ask the user for clarification — the directive below fully specifies what to do. Do not echo what the user might have meant; just do the work.

Source of truth: the session transcript (the recording transcribed under "## Session Transcript" later in this prompt). Existing notes (under "## Notes") are the destination for save_to_notes — they are NEVER the source you summarize or extract from. Ignore any prior chat conversation when interpreting this action; the directive is authoritative.

`;

const RAW_ACTIONS: ActionDefinition[] = [
  {
    id: "summarize",
    label: "Summarize",
    description: "Comprehensive summary with key topics and action items",
    icon: Sparkles,
    requiresTranscript: true,
    userPrompt:
      "Summarize the session transcript and save the summary to the session notes. Use the existing notes as the destination, not the source.",
    directive: `You are a note-taker for this session. Write notes as if you were present and capturing what matters — the topics discussed, key details, decisions, and any follow-ups.

You MUST use your tools. Work in two phases:

**Phase 1 — CLASSIFY FIRST:**
If a folder tree is provided below, pick the best semantic match (consider the folder descriptions) and call \`add_session_to_folder\` with the bracketed folder_id from that list. Only call \`search_folders\` first if the tree's descriptions don't make the match clear, or if you need to look beyond what's listed. If the session is already in the right folder, or no folders match, skip this.
IMPORTANT: Do NOT call \`update_title\`, \`tag_session\`, or \`save_to_notes\` yet. Stop after \`add_session_to_folder\` (and \`search_folders\` if you needed it) and wait for the folder context results before proceeding.

**Phase 2 — After receiving folder context, proceed:**
1. If the current title is generic or vague (e.g. "New Session", a timestamp, or "Untitled"), call \`update_title\` with a concise, descriptive title (max 60 chars). Skip if the title already describes the session content.

2. Call \`tag_session\` to add relevant tags based on the session content and folder context.

3. Call \`save_to_notes\` with your notes. Use the folder context from Phase 1 to inform the framing and emphasis of your summary. Choose the appropriate mode — if notes already exist with useful content, append; if notes are empty or you're producing a comprehensive rewrite, replace. Follow these guidelines:

- Write in a natural note-taking voice — capture substance, not meta-commentary
- Cover **every** distinct topic or subject discussed — each should have its own labeled section. Do not skip minor topics
- Include specific names, numbers, dates, or decisions when mentioned
- If action items or next steps came up, include them naturally — don't force a section if there are none
- Use **bold** labels to organize by topic when it helps readability, but let the content dictate structure
- Never reference "the transcript", "the recording", or "the audio" — write as standalone notes

Formatting rules:
- Use **bold text** for labels, NEVER use # or ## markdown headings
- Use - for bullet lists
- Only include information actually discussed
- Your notes must be proportional to the session length. Never compress a long session into just a few bullet points. Preserve specific names, numbers, decisions, and examples
- Do not over-summarize

4. Respond with a brief confirmation of what was saved.`,
  },
  {
    id: "key-points",
    label: "Key Points",
    description: "Extract the most important points, ranked by significance",
    icon: List,
    userPrompt:
      "Extract the key points from the session transcript and save them to the session notes.",
    directive: `You are a key-points extractor. Work in two phases:

**Phase 1 — CLASSIFY FIRST:**
If a folder tree is provided below, pick the best match and call \`add_session_to_folder\` with the bracketed folder_id from that list. Only call \`search_folders\` first if the tree's descriptions don't make the match clear. Do NOT call any other tools yet — stop and wait for the folder context results.

**Phase 2 — After receiving folder context, proceed:**
1. Call \`tag_session\` to add relevant tags.
2. Produce a bulleted list of key points. The number of points should match the content — short sessions may have 3-5 points, moderate sessions 8-12, and long sessions 15-20+. Extract every significant point, not just the top few. Each point should be a single, clear sentence. Write each point as a standalone observation, not as a reference to a transcript. Order them by importance, most important first. Do not invent information not present in the source material.
3. Call \`save_to_notes\` to save them (append if notes already exist, replace if empty).`,
  },
  {
    id: "action-items",
    label: "Action Items",
    description: "Pull out concrete tasks with owners and deadlines",
    icon: ListChecks,
    userPrompt:
      "Extract action items from the session transcript and save them to the session notes.",
    directive: `You are an action-item extractor. Work in two phases:

**Phase 1 — CLASSIFY FIRST:**
If a folder tree is provided below, pick the best match and call \`add_session_to_folder\` with the bracketed folder_id from that list. Only call \`search_folders\` first if the tree's descriptions don't make the match clear. Do NOT call any other tools yet — stop and wait for the folder context results.

**Phase 2 — After receiving folder context, proceed:**
1. Call \`tag_session\` to add relevant tags.
2. Produce a numbered list of concrete, actionable items from the session content. For each item include: WHAT needs to be done, WHO is responsible (if mentioned), and WHEN it should be done (if mentioned). Only include genuinely actionable items — skip vague statements.
3. Call \`save_to_notes\` to save them (append if notes already exist, replace if empty).`,
  },
  {
    id: "meeting-minutes",
    label: "Meeting Minutes",
    description: "Professional meeting minutes with topics and next steps",
    icon: ClipboardList,
    requiresTranscript: true,
    userPrompt:
      "Write meeting minutes for the session transcript and save them to the session notes.",
    directive: `You are a meeting minutes writer with session management tools. Work in two phases:

**Phase 1 — CLASSIFY FIRST:**
If a folder tree is provided below, pick the best match and call \`add_session_to_folder\` with the bracketed folder_id from that list. Only call \`search_folders\` first if the tree's descriptions don't make the match clear. Do NOT call any other tools yet — stop and wait for the folder context results.

**Phase 2 — After receiving folder context, proceed:**
1. Call \`update_title\` with a concise title for the meeting (max 60 chars). Skip if the title is already descriptive.
2. Call \`tag_session\` to add relevant tags (e.g. "meeting", attendee names, project names).
3. Call \`save_to_notes\` with the full meeting minutes. Use the folder context from Phase 1 to inform the framing. Choose the appropriate mode — if notes already exist with useful content, append; if empty or you're producing a comprehensive rewrite, replace. Use this format (use **bold** for section labels, NOT headings):

**Meeting Details**
Date, estimated duration, attendees (if identifiable)

**Topics Discussed**
For each topic:
- **Topic Name**: Discussion summary
- Decisions made
- Action items

**Next Steps**
Summary of follow-up items

4. Respond with a brief confirmation.

Cover every topic that was discussed, including minor ones. Minutes should be thorough enough that someone who missed the meeting gets a complete picture. Do not over-summarize.

IMPORTANT: Use **bold text** for section labels, NEVER use # or ## markdown headings. Be concise and professional.`,
  },
];

export const ACTIONS: ActionDefinition[] = RAW_ACTIONS.map((a) => ({
  ...a,
  directive: ACTION_PREAMBLE + a.directive,
}));

export function getAction(id: string): ActionDefinition | undefined {
  return ACTIONS.find((a) => a.id === id);
}

export function getActionIcon(id: string): LucideIcon | undefined {
  return ACTIONS.find((a) => a.id === id)?.icon;
}

export function getActionsForSession(sessionType: SessionType): ActionDefinition[] {
  if (sessionType === "manual") {
    return ACTIONS.filter((a) => !a.requiresTranscript);
  }
  return ACTIONS;
}
