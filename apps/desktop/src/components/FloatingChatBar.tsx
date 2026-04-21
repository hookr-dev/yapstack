import { useState, useRef, useEffect, useCallback, useMemo } from "react";
import { useClickOutside } from "@/hooks/useClickOutside";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Button } from "@/components/ui/button";
import {
  Settings,
  ChevronUp,
  ChevronDown,
  Trash2,
} from "lucide-react";
import { AIChatMessage } from "@/components/AIChatMessage";
import { ChatInputBar } from "@/components/chat/ChatInputBar";
import { useAIContext } from "@/components/AIContextProvider";
import { useAppStore } from "@/stores/appStore";
import { useChatMessages } from "@/hooks/useChatMessages";
import {
  getNote,
  saveNote,
} from "@/lib/db";
import {
  isAIConfigured,
  markdownToBasicHtml,
  DEFAULT_AI_SETTINGS,
} from "@/lib/ai";
import type { FileAttachment } from "@/lib/ai";
import { toast } from "sonner";

export function FloatingChatBar() {
  const ctx = useAIContext();

  const [input, setInput] = useState("");
  const [attachments, setAttachments] = useState<FileAttachment[]>([]);
  const [isExpanded, setIsExpanded] = useState(false);
  const [actionsOpen, setActionsOpen] = useState(false);

  const collapseChat = useCallback(() => setIsExpanded(false), []);
  const containerRef = useClickOutside<HTMLDivElement>(
    collapseChat,
    isExpanded,
  );
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const aiSettings = useAppStore((s) => s.settings.ai) ?? DEFAULT_AI_SETTINGS;
  const navigateTo = useAppStore((s) => s.navigateTo);
  const setPlaybackTime = useAppStore((s) => s.setPlaybackTime);
  const incrementNoteRefresh = useAppStore((s) => s.incrementNoteRefresh);
  const aiConfigured = isAIConfigured(aiSettings);

  const sources = useMemo(() => ctx?.sources ?? [], [ctx?.sources]);
  const toggleSource = ctx?.toggleSource;
  const actions = ctx?.actions ?? [];
  const segments = useMemo(() => ctx?.segments ?? [], [ctx?.segments]);
  const isSessionContext = ctx?.isSessionContext ?? false;
  const sessionId = ctx?.sessionId ?? null;

  const { messages, isStreaming, handleSend, handleClearChat } =
    useChatMessages(ctx, input, setInput, attachments, setIsExpanded);

  // Reset local state on context change
  useEffect(() => {
    if (!ctx?.contextKey) return;
    setIsExpanded(false);
    setInput("");
    setAttachments([]);
  }, [ctx?.contextKey]);

  // Toggle chat via keyboard shortcut
  useEffect(() => {
    const handler = () => setIsExpanded((prev) => !prev);
    window.addEventListener("yapstack:toggle-chat", handler);
    return () => window.removeEventListener("yapstack:toggle-chat", handler);
  }, []);

  // Auto-scroll on new messages
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  const handleCitationClick = useCallback(
    (segmentId: string) => {
      const seg = segments.find((s) => s.id === segmentId);
      if (seg) {
        setPlaybackTime(seg.audio_offset_seconds);
        const audioEl = document.querySelector<HTMLAudioElement>(
          "audio[data-session-audio]",
        );
        if (audioEl) {
          audioEl.currentTime = seg.audio_offset_seconds;
          if (audioEl.paused) {
            audioEl.play().catch(() => {});
          }
        }
      }
    },
    [segments, setPlaybackTime],
  );

  const handleSaveToNotes = useCallback(
    async (content: string) => {
      if (!isSessionContext || !sessionId) return;
      try {
        const html = markdownToBasicHtml(content);
        const note = await getNote(sessionId);
        let mergedHtml: string;
        if (note && note.content && note.content !== "<p></p>") {
          mergedHtml = note.content + "<hr>" + html;
        } else {
          mergedHtml = html;
        }
        await saveNote(sessionId, mergedHtml);
        incrementNoteRefresh();
        toast.success("Saved to notes");
      } catch (e) {
        console.error("Failed to save to notes:", e);
        toast.error("Failed to save to notes");
      }
    },
    [isSessionContext, sessionId, incrementNoteRefresh],
  );

  // Guard: no context provided
  if (!ctx) return null;

  // Compact "no API key" inline message
  if (!aiConfigured) {
    return (
      <div className="absolute bottom-2 inset-x-2 z-10 bg-card/95 backdrop-blur-sm border rounded-xl shadow-lg">
        <div className="flex items-center gap-2 px-2 py-2.5">
          <Settings className="h-4 w-4 text-muted-foreground/60 shrink-0" />
          <span className="text-xs text-muted-foreground">
            Configure AI in{" "}
            <button
              className="underline hover:text-foreground"
              onClick={() => navigateTo("settings")}
            >
              Settings
            </button>
          </span>
        </div>
      </div>
    );
  }

  const placeholderText = ctx.placeholder;

  return (
    <div
      ref={containerRef}
      className="absolute bottom-2 inset-x-2 z-10 bg-card/95 backdrop-blur-sm border rounded-xl shadow-lg"
    >
      <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
        {messages.length > 0 && (
          <CollapsibleTrigger asChild>
            <button className="absolute -top-[14px] left-1/2 -translate-x-1/2 z-10 flex items-center justify-center h-3.5 w-7 rounded-t-sm bg-card/95 border border-b-0 text-muted-foreground hover:text-foreground transition-colors">
              {isExpanded ? (
                <ChevronDown className="h-3 w-3" />
              ) : (
                <ChevronUp className="h-3 w-3" />
              )}
            </button>
          </CollapsibleTrigger>
        )}

        <CollapsibleContent>
          <div className="flex items-center justify-end px-2 pt-2">
            <Button
              variant="inline-destructive"
              size="inline"
              onClick={handleClearChat}
              disabled={isStreaming}
            >
              <Trash2 className="h-3 w-3" />
              Clear
            </Button>
          </div>
          <div
            className="max-h-[40vh] overflow-y-auto overscroll-contain select-text"
            ref={scrollRef}
          >
            <div className="p-3 space-y-4">
              {messages.map((msg) => (
                <AIChatMessage
                  key={msg.id}
                  message={msg}
                  onSaveToNotes={
                    msg.role === "assistant" && isSessionContext
                      ? handleSaveToNotes
                      : undefined
                  }
                  segments={isSessionContext ? segments : undefined}
                  onCitationClick={
                    isSessionContext ? handleCitationClick : undefined
                  }
                />
              ))}
            </div>
          </div>
        </CollapsibleContent>

        <ChatInputBar
          input={input}
          setInput={setInput}
          isStreaming={isStreaming}
          onSend={handleSend}
          actions={actions}
          sources={sources}
          toggleSource={toggleSource}
          attachments={attachments}
          setAttachments={setAttachments}
          actionsOpen={actionsOpen}
          setActionsOpen={setActionsOpen}
          placeholderText={placeholderText}
          messagesExist={messages.length > 0}
          isExpanded={isExpanded}
          setIsExpanded={setIsExpanded}
        />
      </Collapsible>
    </div>
  );
}
