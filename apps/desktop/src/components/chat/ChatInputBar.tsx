import { useRef, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import {
  Popover,
  PopoverTrigger,
  PopoverContent,
} from "@/components/ui/popover";
import {
  Command,
  CommandInput,
  CommandList,
  CommandEmpty,
  CommandGroup,
  CommandItem,
} from "@/components/ui/command";
import {
  Send,
  Paperclip,
  Plus,
  X,
  Loader2,
  Wand2,
} from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { readTextFile } from "@tauri-apps/plugin-fs";
import type { FileAttachment } from "@/lib/ai";
import type { ActionDefinition } from "@/lib/ai-actions";
import type { ContextSource } from "@/lib/ai-context";
import { ContextPill } from "./ContextPill";
import { ChatProfilePickerPill } from "./ChatProfilePickerPill";
import { useAppStore } from "@/stores/appStore";
import { toast } from "sonner";

const MAX_ATTACHMENT_BYTES = 500 * 1024;

interface ChatInputBarProps {
  input: string;
  setInput: (value: string) => void;
  isStreaming: boolean;
  onSend: (actionDef?: ActionDefinition) => void;
  actions: ActionDefinition[];
  sources: ContextSource[];
  toggleSource?: (sourceId: string) => void;
  attachments: FileAttachment[];
  setAttachments: React.Dispatch<React.SetStateAction<FileAttachment[]>>;
  actionsOpen: boolean;
  setActionsOpen: (open: boolean) => void;
  placeholderText: string;
  messagesExist: boolean;
  isExpanded: boolean;
  setIsExpanded: (value: boolean) => void;
  /** Chat context key (used by the per-conversation Profile picker). */
  contextKey: string;
}

export function ChatInputBar({
  input,
  setInput,
  isStreaming,
  onSend,
  actions,
  sources,
  toggleSource,
  attachments,
  setAttachments,
  actionsOpen,
  setActionsOpen,
  placeholderText,
  messagesExist,
  isExpanded,
  setIsExpanded,
  contextKey,
}: ChatInputBarProps) {
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const selectedSegmentCount = useAppStore(
    (s) => s.selectedSegmentIds.size,
  );

  const resizeTextarea = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
  }, []);

  const handleInputChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      setInput(e.target.value);
      resizeTextarea();
    },
    [setInput, resizeTextarea],
  );

  useEffect(() => {
    if (!input) resizeTextarea();
  }, [input, resizeTextarea]);

  const handleFocus = useCallback(() => {
    if (messagesExist && !isExpanded) {
      setIsExpanded(true);
    }
  }, [messagesExist, isExpanded, setIsExpanded]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        if (!isStreaming && input.trim()) {
          onSend();
        }
      }
    },
    [isStreaming, input, onSend],
  );

  const handleAttachFile = useCallback(async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [
          {
            name: "Text Files",
            extensions: [
              "txt", "md", "csv", "json", "xml", "log", "yaml", "yml", "toml",
            ],
          },
        ],
      });
      if (!selected) return;
      const path = selected;
      const content = await readTextFile(path);
      if (content.length > MAX_ATTACHMENT_BYTES) {
        toast.error("File too large (max 500 KB)");
        return;
      }
      const name = path.split("/").pop() ?? path.split("\\").pop() ?? "file";
      setAttachments((prev) => [...prev, { name, content }]);
    } catch (e) {
      console.error("Failed to attach file:", e);
    }
  }, [setAttachments]);

  return (
    <>
      {/* Input bar */}
      <div className="flex items-end gap-1 px-2 py-2.5">
        {actions.length > 0 && (
          <Popover open={actionsOpen} onOpenChange={setActionsOpen}>
            <PopoverTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="shrink-0 h-8 w-6 p-0 -ml-0.5 rounded-md text-muted-foreground hover:text-foreground"
                disabled={isStreaming}
              >
                <Wand2 className="h-4 w-4" />
              </Button>
            </PopoverTrigger>
            <PopoverContent
              side="top"
              align="start"
              className="w-72 p-0"
              sideOffset={8}
            >
              <Command>
                <CommandInput
                  placeholder="Search actions..."
                  className="h-9 text-xs"
                />
                <CommandList>
                  <CommandEmpty>No actions found.</CommandEmpty>
                  <CommandGroup>
                    {actions.map((actionDef) => {
                      const Icon = actionDef.icon;
                      return (
                        <CommandItem
                          key={actionDef.id}
                          onSelect={() => {
                            setActionsOpen(false);
                            onSend(actionDef);
                          }}
                        >
                          <Icon className="h-4 w-4 text-muted-foreground" />
                          <div className="flex flex-col">
                            <span className="text-xs font-medium">
                              {actionDef.label}
                            </span>
                            <span className="text-[10px] text-muted-foreground">
                              {actionDef.description}
                            </span>
                          </div>
                        </CommandItem>
                      );
                    })}
                  </CommandGroup>
                </CommandList>
              </Command>
            </PopoverContent>
          </Popover>
        )}

        <Textarea
          ref={textareaRef}
          placeholder={placeholderText}
          value={input}
          onChange={handleInputChange}
          onFocus={handleFocus}
          onKeyDown={handleKeyDown}
          className="min-h-[32px] text-xs resize-none flex-1 transition-[height] duration-100 overflow-hidden"
          rows={1}
        />
        <Button
          variant="secondary"
          size="icon"
          disabled={isStreaming || !input.trim()}
          onClick={() => onSend()}
          className="shrink-0 h-7 w-7 self-center rounded-md"
        >
          {isStreaming ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Send className="h-3.5 w-3.5 -translate-x-[0.5px] translate-y-[0.5px]" />
          )}
        </Button>
      </div>

      {/* Context pills */}
      <div className="flex items-center gap-1.5 px-2 pb-2.5">
        <ChatProfilePickerPill contextKey={contextKey} />
        <span className="text-muted-foreground/30 text-[9px] select-none">|</span>
        {sources.map((source) => (
          <ContextPill
            key={source.id}
            enabled={source.enabled}
            onToggle={source.toggleable && toggleSource ? () => toggleSource(source.id) : undefined}
            icon={<source.icon className="h-2.5 w-2.5" />}
            label={source.label}
            suffix={
              source.type === "transcript" && selectedSegmentCount > 0
                ? `• ${selectedSegmentCount} selected`
                : undefined
            }
          />
        ))}
        {attachments.map((att, i) => (
          <span
            key={i}
            className="inline-flex items-center gap-1 rounded-md border border-primary/30 bg-primary/5 px-2 py-0.5 text-[9px] text-foreground"
          >
            <Paperclip className="h-2.5 w-2.5" />
            {att.name}
            <button
              onClick={() =>
                setAttachments((prev) => prev.filter((_, j) => j !== i))
              }
              className="ml-0.5 hover:text-foreground text-muted-foreground"
            >
              <X className="h-2.5 w-2.5" />
            </button>
          </span>
        ))}
        <button
          onClick={handleAttachFile}
          disabled={isStreaming}
          className="inline-flex items-center gap-1 rounded-md border border-dashed border-muted-foreground/30 px-2 py-0.5 text-[9px] text-muted-foreground hover:border-foreground/40 hover:text-foreground transition-colors disabled:opacity-50"
        >
          <Plus className="h-2.5 w-2.5" />
          Add file
        </button>
      </div>
    </>
  );
}
