import { useEffect, useRef, useCallback } from "react";
import { useEditor, EditorContent } from "@tiptap/react";
import { BubbleMenu } from "@tiptap/react/menus";
import StarterKit from "@tiptap/starter-kit";
import { Markdown } from "@tiptap/markdown";
import type { JSONContent } from "@tiptap/core";
import type { Slice } from "@tiptap/pm/model";
import Placeholder from "@tiptap/extension-placeholder";
import TaskList from "@tiptap/extension-task-list";
import TaskItem from "@tiptap/extension-task-item";
import Highlight from "@tiptap/extension-highlight";
import Underline from "@tiptap/extension-underline";
import Link from "@tiptap/extension-link";
import Typography from "@tiptap/extension-typography";
import {
  Bold,
  Italic,
  Underline as UnderlineIcon,
  Strikethrough,
  List,
  ListOrdered,
  ListChecks,
  Quote,
  Code,
  Highlighter,
  ChevronDown,
  Link as LinkIcon,
  ALargeSmall,
} from "lucide-react";
import type { Editor } from "@tiptap/react";
import {
  saveNote,
  getNote,
  getSessionSegments,
} from "@/lib/db";
import { SegmentReference } from "@/lib/tiptap-segment-ref";
import { convertCitationsToSegmentRefs } from "@/lib/ai-tools";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

function ToolbarButton({
  onClick,
  isActive,
  tooltip,
  children,
}: {
  onClick: () => void;
  isActive: boolean;
  tooltip: string;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-xs"
          onClick={onClick}
          data-active={isActive || undefined}
          className="data-[active]:bg-accent data-[active]:text-accent-foreground"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{tooltip}</TooltipContent>
    </Tooltip>
  );
}

function BubbleButton({
  onClick,
  isActive,
  children,
}: {
  onClick: () => void;
  isActive: boolean;
  children: React.ReactNode;
}) {
  return (
    <Button
      variant="ghost"
      size="icon-xs"
      onClick={onClick}
      data-active={isActive || undefined}
      className="h-7 w-7 data-[active]:bg-accent data-[active]:text-accent-foreground"
    >
      {children}
    </Button>
  );
}

function HeadingDropdown({
  editor,
  size = "toolbar",
}: {
  editor: Editor;
  size?: "toolbar" | "bubble";
}) {
  return (
    <DropdownMenu>
      {size === "toolbar" ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="xs"
                className="gap-0.5 text-xs font-medium"
              >
                <ALargeSmall className="h-4 w-4" />
                <ChevronDown className="h-3 w-3 opacity-50" />
              </Button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent side="bottom">Heading level</TooltipContent>
        </Tooltip>
      ) : (
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon-xs"
            className="h-7 w-7"
          >
            <ALargeSmall className="h-3.5 w-3.5" />
          </Button>
        </DropdownMenuTrigger>
      )}
      <DropdownMenuContent align="start" portal={size === "toolbar"}>
        <DropdownMenuItem
          className="text-xs"
          onClick={() => editor.chain().focus().setParagraph().run()}
        >
          Normal text
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-base font-bold"
          onClick={() => editor.chain().focus().toggleHeading({ level: 1 }).run()}
        >
          Heading 1
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-sm font-semibold"
          onClick={() => editor.chain().focus().toggleHeading({ level: 2 }).run()}
        >
          Heading 2
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-[13px] font-medium"
          onClick={() => editor.chain().focus().toggleHeading({ level: 3 }).run()}
        >
          Heading 3
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-xs font-medium"
          onClick={() => editor.chain().focus().toggleHeading({ level: 4 }).run()}
        >
          Heading 4
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function NoteEditor({
  sessionId,
  refreshKey,
  onSeekTime,
}: {
  sessionId: string;
  refreshKey?: number;
  onSeekTime?: (seconds: number) => void;
}) {
  const lastSavedContent = useRef<string>("");
  const saveTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const editor: Editor | null = useEditor({
    autofocus: "end",
    extensions: [
      StarterKit,
      Markdown,
      Placeholder.configure({
        placeholder: "Write your notes here...",
      }),
      TaskList,
      TaskItem.configure({ nested: true }),
      Highlight.configure({ multicolor: false }),
      Underline,
      Link.configure({
        openOnClick: false,
        autolink: true,
        HTMLAttributes: { class: "text-primary underline" },
      }),
      Typography,
      SegmentReference,
    ],
    content: "",
    editorProps: {
      attributes: {
        class:
          "tiptap-editor max-w-none focus:outline-none min-h-[200px] px-4 py-3 text-sm",
      },
      clipboardTextSerializer: (slice: Slice) => {
        const nodes: JSONContent[] = [];
        slice.content.forEach((node) => nodes.push(node.toJSON()));
        const manager = editor?.storage.markdown?.manager;
        if (!manager) return slice.content.textBetween(0, slice.content.size);
        return manager.serialize({ type: "doc", content: nodes });
      },
    },
    onUpdate: ({ editor }) => {
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }
      saveTimeoutRef.current = setTimeout(() => {
        const html = editor.getHTML();
        saveNote(sessionId, html).catch((e) =>
          console.error("Failed to save note:", e),
        );
      }, 1000);
    },
  });

  // Load note content on mount and when refreshKey changes
  useEffect(() => {
    async function load() {
      if (!editor) return;
      const note = await getNote(sessionId);
      let content = note?.content ?? "";
      // Convert [[seg:ID]] text citations to <span data-segment-ref> nodes
      if (content.includes("[[seg:")) {
        const segments = await getSessionSegments(sessionId);
        content = convertCitationsToSegmentRefs(content, segments);
        // Persist the converted content so future loads don't need re-conversion
        if (note && content !== note.content) {
          await saveNote(sessionId, content);
        }
      }
      editor.commands.setContent(content);
      lastSavedContent.current = content;
    }
    load();
  }, [sessionId, editor, refreshKey]);

  // Create version snapshot on blur if content changed
  const handleBlur = useCallback(async () => {
    if (!editor) return;
    const html = editor.getHTML();

    await saveNote(sessionId, html);

    lastSavedContent.current = html;
  }, [editor, sessionId]);

  useEffect(() => {
    if (!editor) return;
    editor.on("blur", handleBlur);
    return () => {
      editor.off("blur", handleBlur);
    };
  }, [editor, handleBlur]);

  // Cleanup timeout on unmount
  useEffect(() => {
    return () => {
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }
    };
  }, []);

  // Listen for segment reference insertion events
  useEffect(() => {
    const handleInsertRef = (e: Event) => {
      if (!editor) return;
      const detail = (e as CustomEvent).detail;
      editor
        .chain()
        .focus()
        .insertContent({
          type: "segmentReference",
          attrs: {
            segmentId: detail.segmentId,
            timestamp: detail.timestamp,
            offsetSeconds: detail.offsetSeconds,
          },
        })
        .insertContent(" ")
        .run();
    };
    window.addEventListener("yapstack:insert-segment-ref", handleInsertRef);
    return () => window.removeEventListener("yapstack:insert-segment-ref", handleInsertRef);
  }, [editor]);

  // Listen for seek events from segment reference clicks
  useEffect(() => {
    if (!onSeekTime) return;
    const handleSeek = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      onSeekTime(detail.offsetSeconds);
    };
    window.addEventListener("yapstack:seek-segment", handleSeek);
    return () => window.removeEventListener("yapstack:seek-segment", handleSeek);
  }, [onSeekTime]);

  // Bubble menu link handler
  const handleSetLink = useCallback(() => {
    if (!editor) return;
    const previousUrl = editor.getAttributes("link").href ?? "";
    const url = window.prompt("URL", previousUrl);
    if (url === null) return;
    if (url === "") {
      editor.chain().focus().extendMarkRange("link").unsetLink().run();
    } else {
      editor.chain().focus().extendMarkRange("link").setLink({ href: url }).run();
    }
  }, [editor]);

  if (!editor) return null;

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <div className="flex items-center gap-0.5 border-b bg-muted/30 px-3 py-1.5">
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleBold().run()}
          isActive={editor.isActive("bold")}
          tooltip="Bold"
        >
          <Bold />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleItalic().run()}
          isActive={editor.isActive("italic")}
          tooltip="Italic"
        >
          <Italic />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleUnderline().run()}
          isActive={editor.isActive("underline")}
          tooltip="Underline"
        >
          <UnderlineIcon />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleStrike().run()}
          isActive={editor.isActive("strike")}
          tooltip="Strikethrough"
        >
          <Strikethrough />
        </ToolbarButton>
        <div className="mx-1 h-4 w-px bg-border" />
        <HeadingDropdown editor={editor} size="toolbar" />
        <div className="mx-1 h-4 w-px bg-border" />
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleBulletList().run()}
          isActive={editor.isActive("bulletList")}
          tooltip="Bullet List"
        >
          <List />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleOrderedList().run()}
          isActive={editor.isActive("orderedList")}
          tooltip="Ordered List"
        >
          <ListOrdered />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleTaskList().run()}
          isActive={editor.isActive("taskList")}
          tooltip="Checklist"
        >
          <ListChecks />
        </ToolbarButton>
        <div className="mx-1 h-4 w-px bg-border" />
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleBlockquote().run()}
          isActive={editor.isActive("blockquote")}
          tooltip="Blockquote"
        >
          <Quote />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleCode().run()}
          isActive={editor.isActive("code")}
          tooltip="Code"
        >
          <Code />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleHighlight().run()}
          isActive={editor.isActive("highlight")}
          tooltip="Highlight"
        >
          <Highlighter />
        </ToolbarButton>
        <div className="flex-1" />
      </div>
      <BubbleMenu
        editor={editor}
        shouldShow={({ editor: e, state: s }) => {
          const { from, to } = s.selection;
          if (from === to) return false;
          if (e.isActive("codeBlock")) return false;
          let hasSegRef = false;
          s.doc.nodesBetween(from, to, (node) => {
            if (node.type.name === "segmentReference") hasSegRef = true;
            return !hasSegRef;
          });
          if (hasSegRef) return false;
          return true;
        }}
        options={{ placement: "top", offset: { mainAxis: 12 } }}
      >
        <div className="flex items-center gap-0.5 rounded-lg border bg-popover p-1 shadow-lg">
          <BubbleButton
            onClick={() => editor.chain().focus().toggleBold().run()}
            isActive={editor.isActive("bold")}
          >
            <Bold className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={() => editor.chain().focus().toggleItalic().run()}
            isActive={editor.isActive("italic")}
          >
            <Italic className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={() => editor.chain().focus().toggleUnderline().run()}
            isActive={editor.isActive("underline")}
          >
            <UnderlineIcon className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={() => editor.chain().focus().toggleStrike().run()}
            isActive={editor.isActive("strike")}
          >
            <Strikethrough className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={() => editor.chain().focus().toggleCode().run()}
            isActive={editor.isActive("code")}
          >
            <Code className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={() => editor.chain().focus().toggleHighlight().run()}
            isActive={editor.isActive("highlight")}
          >
            <Highlighter className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={handleSetLink}
            isActive={editor.isActive("link")}
          >
            <LinkIcon className="h-3.5 w-3.5" />
          </BubbleButton>
          <div className="mx-0.5 h-4 w-px bg-border" />
          <HeadingDropdown editor={editor} size="bubble" />
        </div>
      </BubbleMenu>
      <div className="flex-1 overflow-auto pb-16 select-text">
        <EditorContent editor={editor} />
      </div>
    </div>
  );
}
