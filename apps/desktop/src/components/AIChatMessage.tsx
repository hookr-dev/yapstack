import ReactMarkdown from "react-markdown";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Copy, Check, BookmarkPlus, Quote } from "lucide-react";
import { memo, useState, type ReactNode } from "react";
import type { ChatMessage } from "@/lib/ai";
import type { DbSegment } from "@/lib/db";
import { getAction } from "@/lib/ai-actions";
import { ToolExecutionBlock } from "@/components/ToolExecutionBlock";

interface ToolBadge {
  tool: string;
  detail: string;
}

interface AIChatMessageProps {
  message: ChatMessage;
  onSaveToNotes?: (content: string) => void;
  segments?: DbSegment[];
  onCitationClick?: (segmentId: string) => void;
}

const TOOL_LABELS: Record<string, string> = {
  update_title: "Title updated",
  save_to_notes: "Notes saved",
  pin_session: "Pin toggled",
  tag_session: "Tags updated",
  add_session_to_folder: "Classified",
  search_folders: "Folders searched",
  search_sessions: "Sessions searched",
  get_session_context: "Sessions expanded",
  search_dictations: "Dictations searched",
  replace_in_transcript: "Transcript edited",
};

function parseToolBadges(content: string): {
  badges: ToolBadge[];
  text: string;
} {
  const lines = content.split("\n");
  const badges: ToolBadge[] = [];
  let textStart = 0;

  for (const line of lines) {
    const match = line.match(/^\[tool:(\w+)\]\s*(.*)$/);
    if (match) {
      badges.push({ tool: match[1], detail: match[2] });
      textStart++;
    } else {
      break;
    }
  }

  return {
    badges,
    text: lines.slice(textStart).join("\n").trimStart(),
  };
}

function formatTimestamp(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

// Stateless pattern used for `.test()` checks. `renderTextWithCitations`
// builds its own `g`-flagged regex from `.source` for the exec-loop, so
// keeping this one without `g` is safe and prevents the cross-call
// `lastIndex` drift that was making citations render as raw text after
// the first chat message in a session.
const CITE_REGEX = /\[\[seg:([a-zA-Z0-9_-]+)\]\]/;

function renderTextWithCitations(
  text: string,
  segments: DbSegment[] | undefined,
  onCitationClick: ((segmentId: string) => void) | undefined,
): ReactNode[] {
  const parts: ReactNode[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  const regex = new RegExp(CITE_REGEX.source, "g");
  while ((match = regex.exec(text)) !== null) {
    // Text before the citation
    if (match.index > lastIndex) {
      parts.push(text.slice(lastIndex, match.index));
    }

    const segId = match[1];
    const segment = segments?.find((s) => s.id === segId);

    if (segment && onCitationClick) {
      const ts = formatTimestamp(segment.audio_offset_seconds);
      const preview =
        segment.text.length > 80
          ? segment.text.slice(0, 80) + "..."
          : segment.text;

      parts.push(
        <Tooltip key={`cite-${match.index}`}>
          <TooltipTrigger asChild>
            <button
              onClick={() => onCitationClick(segId)}
              className="inline-flex items-center gap-0.5 rounded bg-accent px-1 py-0.5 text-[10px] font-medium text-accent-foreground hover:bg-accent/80 cursor-pointer align-baseline"
            >
              <Quote className="h-2.5 w-2.5" />
              {ts}
            </button>
          </TooltipTrigger>
          <TooltipContent side="top" className="max-w-xs text-xs">
            {preview}
          </TooltipContent>
        </Tooltip>,
      );
    } else {
      // Segment not found — render as plain text timestamp or remove marker
      parts.push(
        <span
          key={`cite-${match.index}`}
          className="inline-flex items-center gap-0.5 rounded bg-accent/50 px-1 py-0.5 text-[10px] font-medium text-accent-foreground/60"
        >
          <Quote className="h-2.5 w-2.5" />
          ref
        </span>,
      );
    }

    lastIndex = match.index + match[0].length;
  }

  // Remaining text
  if (lastIndex < text.length) {
    parts.push(text.slice(lastIndex));
  }

  return parts;
}

export const AIChatMessage = memo(function AIChatMessage({
  message,
  onSaveToNotes,
  segments,
  onCitationClick,
}: AIChatMessageProps) {
  const [copied, setCopied] = useState(false);

  const isUser = message.role === "user";

  function handleCopy() {
    navigator.clipboard.writeText(message.content);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  if (isUser) {
    const actionDef = message.action ? getAction(message.action) : undefined;

    if (actionDef) {
      const Icon = actionDef.icon;
      return (
        <div className="flex justify-end">
          <div className="inline-flex items-center gap-1.5 rounded-full bg-primary/10 border border-primary/20 px-3 py-1.5">
            <Icon className="h-3 w-3 text-primary" />
            <span className="text-xs font-medium text-primary">
              {actionDef.label}
            </span>
          </div>
        </div>
      );
    }

    return (
      <div className="flex justify-end">
        <div className="max-w-[85%] rounded-2xl rounded-br-md bg-primary text-primary-foreground px-3 py-2">
          <p className="text-xs whitespace-pre-wrap">{message.content}</p>
        </div>
      </div>
    );
  }

  const hasLiveExecs = message.toolExecutions && message.toolExecutions.length > 0;
  const { badges, text } = parseToolBadges(message.content);
  const hasCitations = CITE_REGEX.test(text);

  const toolExecsFromBadges = !hasLiveExecs && badges.length > 0
    ? badges.map((b) => ({
        name: b.tool,
        label: TOOL_LABELS[b.tool] ?? b.tool,
        detail: b.detail || undefined,
        status: "done" as const,
      }))
    : undefined;

  const toolExecs = message.toolExecutions ?? toolExecsFromBadges;

  return (
    <div className="flex flex-col gap-1">
      {toolExecs && toolExecs.length > 0 && (
        <div className="max-w-[95%] rounded-xl bg-muted/40 border border-border/30 px-2.5 py-1.5">
          <ToolExecutionBlock executions={toolExecs} />
        </div>
      )}
      {(text.trim() || message.isStreaming) && (
        <div className="max-w-[95%] rounded-2xl rounded-bl-md bg-muted/80 border border-border/50 px-3 py-2 text-xs ai-chat-markdown">
          {hasCitations && (segments || onCitationClick) ? (
            <ReactMarkdown
              components={{
                p: ({ children }) => (
                  <p>
                    {processCitationsInChildren(
                      children,
                      segments,
                      onCitationClick,
                    )}
                  </p>
                ),
                li: ({ children }) => (
                  <li>
                    {processCitationsInChildren(
                      children,
                      segments,
                      onCitationClick,
                    )}
                  </li>
                ),
              }}
            >
              {text}
            </ReactMarkdown>
          ) : (
            <ReactMarkdown>{text}</ReactMarkdown>
          )}
          {message.isStreaming && (
            <span className="inline-block w-1.5 h-4 bg-foreground/60 animate-pulse ml-0.5 align-text-bottom" />
          )}
        </div>
      )}
      {!message.isStreaming && text.trim() && (
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon-xs"
            onClick={handleCopy}
            className="h-6 w-6 text-muted-foreground"
          >
            {copied ? (
              <Check className="h-3 w-3" />
            ) : (
              <Copy className="h-3 w-3" />
            )}
          </Button>
          {onSaveToNotes && (
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={() => onSaveToNotes(message.content)}
              className="h-6 w-6 text-muted-foreground"
            >
              <BookmarkPlus className="h-3 w-3" />
            </Button>
          )}
        </div>
      )}
    </div>
  );
});

function processCitationsInChildren(
  children: ReactNode,
  segments: DbSegment[] | undefined,
  onCitationClick: ((segmentId: string) => void) | undefined,
): ReactNode {
  if (!children) return children;

  if (typeof children === "string") {
    if (CITE_REGEX.test(children)) {
      return renderTextWithCitations(children, segments, onCitationClick);
    }
    return children;
  }

  if (Array.isArray(children)) {
    return children.map((child, i) => {
      if (typeof child === "string" && CITE_REGEX.test(child)) {
        return (
          <span key={i}>
            {renderTextWithCitations(child, segments, onCitationClick)}
          </span>
        );
      }
      return child;
    });
  }

  return children;
}
