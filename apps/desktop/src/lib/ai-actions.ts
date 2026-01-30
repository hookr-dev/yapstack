import type { LucideIcon } from "lucide-react";
import { Sparkles, List, ListChecks, ClipboardList } from "lucide-react";

export interface ActionDefinition {
  id: string;
  label: string;
  description: string;
  icon: LucideIcon;
  requiresTranscript?: boolean;
  directive: string;
}

export const ACTIONS: ActionDefinition[] = [
  {
    id: "summarize",
    label: "Summarize",
    description: "Comprehensive summary with key topics and action items",
    icon: Sparkles,
    requiresTranscript: true,
    directive: `You are a note-taker for this session. Write notes as if you were present and capturing what matters — the topics discussed, key details, decisions, and any follow-ups.

You MUST use your tools.

1. If the current title is generic or vague (e.g. "New Session", a timestamp, or "Untitled"), call \`update_title\` with a concise, descriptive title (max 60 chars). If the title already describes the session content, skip this step.

2. Call \`save_to_notes\` with your notes. Choose the appropriate mode — if notes already exist with useful content, append; if notes are empty or you're producing a comprehensive rewrite, replace. Follow these guidelines:

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

3. After calling tools, respond with a brief confirmation of what was saved.`,
  },
  {
    id: "key-points",
    label: "Key Points",
    description: "Extract the most important points, ranked by significance",
    icon: List,
    directive: `You are a key-points extractor. Produce a bulleted list of key points from the session content below. The number of points should match the content — short sessions may have 3-5 points, moderate sessions 8-12, and long sessions 15-20+. Extract every significant point, not just the top few. Each point should be a single, clear sentence. Write each point as a standalone observation, not as a reference to a transcript. Order them by importance, most important first. Do not invent information not present in the source material.

After producing the key points, call \`save_to_notes\` to save them (append if notes already exist, replace if empty).`,
  },
  {
    id: "action-items",
    label: "Action Items",
    description: "Pull out concrete tasks with owners and deadlines",
    icon: ListChecks,
    directive: `You are an action-item extractor. Produce a numbered list of concrete, actionable items from the session content below. For each item include: WHAT needs to be done, WHO is responsible (if mentioned), and WHEN it should be done (if mentioned). Only include genuinely actionable items — skip vague statements.

After producing the action items, call \`save_to_notes\` to save them (append if notes already exist, replace if empty).`,
  },
  {
    id: "meeting-minutes",
    label: "Meeting Minutes",
    description: "Professional meeting minutes with topics and next steps",
    icon: ClipboardList,
    requiresTranscript: true,
    directive: `You are a meeting minutes writer with session management tools. Produce professional meeting minutes from the session content below.

You MUST use your tools:
1. Call \`update_title\` with a concise title for the meeting (max 60 chars). Skip if the title is already descriptive.
2. Call \`save_to_notes\` with the full meeting minutes. Choose the appropriate mode — if notes already exist with useful content, append; if empty or you're producing a comprehensive rewrite, replace. Use this format (use **bold** for section labels, NOT headings):

**Meeting Details**
Date, estimated duration, attendees (if identifiable)

**Topics Discussed**
For each topic:
- **Topic Name**: Discussion summary
- Decisions made
- Action items

**Next Steps**
Summary of follow-up items

3. After calling tools, respond with a brief confirmation.

Cover every topic that was discussed, including minor ones. Minutes should be thorough enough that someone who missed the meeting gets a complete picture. Do not over-summarize.

IMPORTANT: Use **bold text** for section labels, NEVER use # or ## markdown headings. Be concise and professional.`,
  },
];

export function getAction(id: string): ActionDefinition | undefined {
  return ACTIONS.find((a) => a.id === id);
}

export function getActionIcon(id: string): LucideIcon | undefined {
  return ACTIONS.find((a) => a.id === id)?.icon;
}

export function getActionsForSession(sessionType: string): ActionDefinition[] {
  if (sessionType === "manual") {
    return ACTIONS.filter((a) => !a.requiresTranscript);
  }
  return ACTIONS;
}
