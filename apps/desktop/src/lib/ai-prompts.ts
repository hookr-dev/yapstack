import type { FileAttachment } from "./ai";

export function getContentScaleGuidance(transcriptText: string): string {
  const words = transcriptText.split(/\s+/).filter(Boolean).length;
  const segments = (transcriptText.match(/\[seg:/g) || []).length;

  if (words < 150) {
    return `**Content scale:** ~${words} words${segments ? ` across ${segments} segments` : ""}. This is a very short session — capture all topics mentioned.`;
  }

  let ratio: number;
  if (words <= 500) {
    ratio = 0.4;
  } else if (words <= 2000) {
    ratio = 0.3;
  } else {
    ratio = 0.2;
  }

  const target = Math.round(words * ratio);
  return `**Content scale:** ~${words} words${segments ? ` across ${segments} segments` : ""}. Your output should be at most ${target} words. Cover every distinct topic — do not over-condense. Preserve specific names, numbers, decisions, and action items.`;
}

const CITATION_INSTRUCTION = `
When referencing specific parts of the conversation, cite the segment using its ID in this format: [[seg:ID]]. For example: "The team discussed increasing marketing spend [[seg:def456]]." Only cite when it adds value — don't cite every statement.`;

const NOTES_GUIDANCE = `
When saving to notes, choose the mode based on context:
- Use "append" to add your content alongside existing notes
- Use "replace" only when notes are empty or when you're producing a complete rewrite that incorporates the existing content
If existing notes contain useful content (hand-written notes, previously added sections), default to append.`;

export const GENERAL_DIRECTIVE = `You are a helpful assistant for a note-taking app. You have access to the user's session content, notes, and session tools.

Answer questions accurately, referencing specific parts of the conversation or notes when relevant. Be concise and direct.

You can use your tools to help the user:
- \`update_title\` — Set a better session title if the user asks
- \`save_to_notes\` — Save content to notes when the user asks you to write, draft, or create something
- \`pin_session\` — Pin/unpin the session

Only use tools when the user's request clearly calls for it. For general questions, just answer in text.`;

export function getSystemPrompt(
  directive: string,
  transcriptText: string,
  noteText: string,
  attachments: { name: string; content: string }[],
): string {
  let context = directive;
  if (transcriptText) {
    context += "\n" + CITATION_INSTRUCTION;
  }
  context += "\n" + NOTES_GUIDANCE + "\n\n---\n\n";

  if (transcriptText) {
    context += getContentScaleGuidance(transcriptText) + "\n\n";
    context += `## Session Transcript\n${transcriptText}\n\n`;
  }

  if (noteText) {
    context += `## Notes\n${noteText}\n\n`;
  }

  if (attachments.length > 0) {
    context += `## Attached Files\n`;
    for (const att of attachments) {
      context += `### ${att.name}\n${att.content}\n\n`;
    }
  }

  return context;
}

export interface SessionMeta {
  title: string;
  isPinned: boolean;
  hasNotes: boolean;
}

export function getSystemPromptWithToolContext(
  directive: string,
  transcriptText: string,
  noteText: string,
  attachments: { name: string; content: string }[],
  sessionMeta: SessionMeta,
): string {
  const base = getSystemPrompt(directive, transcriptText, noteText, attachments);

  return (
    base +
    `\n---\nSession metadata:\n- Current title: "${sessionMeta.title}"\n- Pinned: ${sessionMeta.isPinned ? "yes" : "no"}\n- Has existing notes: ${sessionMeta.hasNotes ? "yes" : "no"}\n`
  );
}

export interface FolderContextLayer {
  name: string;
  description: string;
}

export function getDictationSystemPrompt(
  dictationContext: string,
  attachments: FileAttachment[],
): string {
  let prompt = `You are a helpful assistant for a voice dictation app. The user is viewing their dictation history — a log of past voice-to-text dictations. You can answer questions, search for content across dictations, find patterns, compare entries, and help the user work with their dictation output.

Be concise and direct. Use **bold** for labels. Do not use # or ## markdown headings.

---

## Dictation History

${dictationContext}
`;

  if (attachments.length > 0) {
    prompt += `\n## Attached Files\n`;
    for (const att of attachments) {
      prompt += `### ${att.name}\n${att.content}\n\n`;
    }
  }

  return prompt;
}

export function getMultiSessionSystemPrompt(
  sessionsContext: string,
  attachments: FileAttachment[],
  folderContext?: FolderContextLayer[],
): string {
  let prompt = `You are a helpful assistant for a note-taking app. The user is viewing multiple sessions. You can answer questions, compare sessions, find patterns, and synthesize information across them.

Be concise and direct. Use **bold** for labels when organizing information. Do not use # or ## markdown headings.

---
`;

  if (folderContext && folderContext.length > 0) {
    prompt += `\n**Organizational context:**\n`;
    for (const layer of folderContext) {
      prompt += `- **${layer.name}:** ${layer.description}\n`;
    }
  }

  prompt += `
## Sessions

${sessionsContext}
`;

  if (attachments.length > 0) {
    prompt += `\n## Attached Files\n`;
    for (const att of attachments) {
      prompt += `### ${att.name}\n${att.content}\n\n`;
    }
  }

  return prompt;
}
