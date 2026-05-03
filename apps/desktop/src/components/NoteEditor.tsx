import { useEffect, useRef, useCallback } from "react";
import { useEditor, EditorContent, useEditorState } from "@tiptap/react";
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
  SquareCode,
  Highlighter,
  ChevronDown,
  Link as LinkIcon,
  ALargeSmall,
  Eraser,
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

// `value` is the literal string TipTap writes into mark inline `style="background-color: ..."`,
// so a `var(...)` reference resolves per theme at render time.
const HIGHLIGHT_COLORS = [
  { name: "Yellow", value: "var(--tt-color-highlight-yellow)" },
  { name: "Green", value: "var(--tt-color-highlight-green)" },
  { name: "Blue", value: "var(--tt-color-highlight-blue)" },
  { name: "Purple", value: "var(--tt-color-highlight-purple)" },
  { name: "Red", value: "var(--tt-color-highlight-red)" },
] as const;

type HighlightColor = (typeof HIGHLIGHT_COLORS)[number]["value"];

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
          className="relative data-[active]:bg-accent data-[active]:text-accent-foreground data-[active]:after:absolute data-[active]:after:bottom-0.5 data-[active]:after:left-1/2 data-[active]:after:h-0.5 data-[active]:after:w-3 data-[active]:after:-translate-x-1/2 data-[active]:after:rounded-full data-[active]:after:bg-accent-foreground"
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
      className="relative h-7 w-7 data-[active]:bg-accent data-[active]:text-accent-foreground data-[active]:after:absolute data-[active]:after:bottom-0.5 data-[active]:after:left-1/2 data-[active]:after:h-0.5 data-[active]:after:w-2.5 data-[active]:after:-translate-x-1/2 data-[active]:after:rounded-full data-[active]:after:bg-accent-foreground"
    >
      {children}
    </Button>
  );
}

function HeadingDropdown({
  editor,
  level,
}: {
  editor: Editor;
  level: 1 | 2 | 3 | 4 | null;
}) {
  const label = level == null ? "Normal" : `H${level}`;
  const isActive = level != null;
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="xs"
              data-active={isActive || undefined}
              className="gap-0.5 text-xs font-medium data-[active]:bg-accent data-[active]:text-accent-foreground"
            >
              <ALargeSmall className="h-4 w-4" />
              <span className="ml-0.5 tabular-nums">{label}</span>
              <ChevronDown className="h-3 w-3 opacity-50" />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="bottom">Heading level</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="start">
        <DropdownMenuItem
          data-active={level == null || undefined}
          className="text-xs data-[active]:bg-accent data-[active]:text-accent-foreground"
          onClick={() => editor.chain().focus().setParagraph().run()}
        >
          Normal text
        </DropdownMenuItem>
        <DropdownMenuItem
          data-active={level === 1 || undefined}
          className="text-base font-bold data-[active]:bg-accent data-[active]:text-accent-foreground"
          onClick={() => editor.chain().focus().toggleHeading({ level: 1 }).run()}
        >
          Heading 1
        </DropdownMenuItem>
        <DropdownMenuItem
          data-active={level === 2 || undefined}
          className="text-sm font-semibold data-[active]:bg-accent data-[active]:text-accent-foreground"
          onClick={() => editor.chain().focus().toggleHeading({ level: 2 }).run()}
        >
          Heading 2
        </DropdownMenuItem>
        <DropdownMenuItem
          data-active={level === 3 || undefined}
          className="text-[13px] font-medium data-[active]:bg-accent data-[active]:text-accent-foreground"
          onClick={() => editor.chain().focus().toggleHeading({ level: 3 }).run()}
        >
          Heading 3
        </DropdownMenuItem>
        <DropdownMenuItem
          data-active={level === 4 || undefined}
          className="text-xs font-medium data-[active]:bg-accent data-[active]:text-accent-foreground"
          onClick={() => editor.chain().focus().toggleHeading({ level: 4 }).run()}
        >
          Heading 4
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function HighlightDropdown({
  editor,
  isActive,
  currentColor,
  size = "toolbar",
}: {
  editor: Editor;
  isActive: boolean;
  currentColor: HighlightColor | null;
  size?: "toolbar" | "bubble";
}) {
  const apply = (color: HighlightColor) => {
    editor.chain().focus().setHighlight({ color }).run();
  };
  const remove = () => {
    editor.chain().focus().unsetHighlight().run();
  };
  const trigger =
    size === "toolbar" ? (
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-xs"
              data-active={isActive || undefined}
              className="data-[active]:bg-accent data-[active]:text-accent-foreground"
            >
              <Highlighter />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="bottom">Highlight</TooltipContent>
      </Tooltip>
    ) : (
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-xs"
          data-active={isActive || undefined}
          className="h-7 w-7 data-[active]:bg-accent data-[active]:text-accent-foreground"
        >
          <Highlighter className="h-3.5 w-3.5" />
        </Button>
      </DropdownMenuTrigger>
    );
  return (
    <DropdownMenu>
      {trigger}
      <DropdownMenuContent align="start" portal={size === "toolbar"} className="min-w-0 p-1">
        <div className="flex items-center gap-1 px-1 py-0.5">
          {HIGHLIGHT_COLORS.map((c) => (
            <button
              key={c.value}
              type="button"
              onClick={() => apply(c.value)}
              aria-label={`Highlight ${c.name.toLowerCase()}`}
              data-active={currentColor === c.value || undefined}
              className="size-5 rounded-full border border-border ring-offset-1 ring-offset-popover transition-shadow hover:ring-2 hover:ring-ring data-[active]:ring-2 data-[active]:ring-ring"
              style={{ background: c.value }}
            />
          ))}
          <div className="mx-0.5 h-5 w-px bg-border" />
          <button
            type="button"
            onClick={remove}
            aria-label="Remove highlight"
            className="flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <Eraser className="h-3 w-3" />
          </button>
        </div>
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
      Highlight.configure({ multicolor: true }),
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
      // When the clipboard payload is plain-text markdown (no rich HTML
      // alternative) and contains a code fence, parse it as markdown so
      // ```code blocks``` and other markdown survive the paste.
      handlePaste: (_view, event) => {
        const text = event.clipboardData?.getData("text/plain");
        if (!text || !text.includes("```")) return false;
        if (event.clipboardData?.getData("text/html")) return false;
        if (!editor) return false;
        editor.commands.insertContent(text, { contentType: "markdown" });
        return true;
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

  // `editor.isActive(...)` is not reactive in React; this hook re-renders on transactions.
  const state = useEditorState({
    editor,
    selector: ({ editor: e }) =>
      e
        ? {
            isBold: e.isActive("bold"),
            isItalic: e.isActive("italic"),
            isUnderline: e.isActive("underline"),
            isStrike: e.isActive("strike"),
            isCode: e.isActive("code"),
            isCodeBlock: e.isActive("codeBlock"),
            isHighlight: e.isActive("highlight"),
            isLink: e.isActive("link"),
            isBulletList: e.isActive("bulletList"),
            isOrderedList: e.isActive("orderedList"),
            isTaskList: e.isActive("taskList"),
            isBlockquote: e.isActive("blockquote"),
            headingLevel: (e.getAttributes("heading").level ?? null) as
              | 1 | 2 | 3 | 4 | null,
            highlightColor:
              (e.getAttributes("highlight").color as HighlightColor | undefined) ?? null,
          }
        : null,
  });

  if (!editor || !state) return null;

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <div className="flex items-center gap-0.5 border-b bg-muted/30 px-3 py-1.5">
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleBold().run()}
          isActive={state.isBold}
          tooltip="Bold"
        >
          <Bold />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleItalic().run()}
          isActive={state.isItalic}
          tooltip="Italic"
        >
          <Italic />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleUnderline().run()}
          isActive={state.isUnderline}
          tooltip="Underline"
        >
          <UnderlineIcon />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleStrike().run()}
          isActive={state.isStrike}
          tooltip="Strikethrough"
        >
          <Strikethrough />
        </ToolbarButton>
        <ToolbarButton onClick={handleSetLink} isActive={state.isLink} tooltip="Link">
          <LinkIcon />
        </ToolbarButton>
        <div className="mx-1 h-4 w-px bg-border" />
        <HeadingDropdown editor={editor} level={state.headingLevel} />
        <div className="mx-1 h-4 w-px bg-border" />
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleBulletList().run()}
          isActive={state.isBulletList}
          tooltip="Bullet List"
        >
          <List />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleOrderedList().run()}
          isActive={state.isOrderedList}
          tooltip="Ordered List"
        >
          <ListOrdered />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleTaskList().run()}
          isActive={state.isTaskList}
          tooltip="Checklist"
        >
          <ListChecks />
        </ToolbarButton>
        <div className="mx-1 h-4 w-px bg-border" />
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleBlockquote().run()}
          isActive={state.isBlockquote}
          tooltip="Blockquote"
        >
          <Quote />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleCode().run()}
          isActive={state.isCode}
          tooltip="Inline code"
        >
          <Code />
        </ToolbarButton>
        <ToolbarButton
          onClick={() => editor.chain().focus().toggleCodeBlock().run()}
          isActive={state.isCodeBlock}
          tooltip="Code block"
        >
          <SquareCode />
        </ToolbarButton>
        <HighlightDropdown
          editor={editor}
          isActive={state.isHighlight}
          currentColor={state.highlightColor}
          size="toolbar"
        />
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
          return !hasSegRef;
        }}
        options={{
          placement: "top",
          offset: { mainAxis: 8 },
          // Bound flip/shift to the editor's contenteditable so the bubble can't
          // escape into the static toolbar above or the FloatingChatBar below.
          flip: {
            boundary: editor.view.dom,
            fallbackPlacements: ["bottom", "top"],
            padding: 8,
          },
          shift: { boundary: editor.view.dom, padding: 8 },
        }}
      >
        <div className="z-50 flex items-center gap-0.5 rounded-lg border bg-popover p-1 shadow-lg">
          <BubbleButton
            onClick={() => editor.chain().focus().toggleBold().run()}
            isActive={state.isBold}
          >
            <Bold className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={() => editor.chain().focus().toggleItalic().run()}
            isActive={state.isItalic}
          >
            <Italic className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={() => editor.chain().focus().toggleUnderline().run()}
            isActive={state.isUnderline}
          >
            <UnderlineIcon className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={() => editor.chain().focus().toggleStrike().run()}
            isActive={state.isStrike}
          >
            <Strikethrough className="h-3.5 w-3.5" />
          </BubbleButton>
          <BubbleButton
            onClick={() => editor.chain().focus().toggleCode().run()}
            isActive={state.isCode}
          >
            <Code className="h-3.5 w-3.5" />
          </BubbleButton>
          <HighlightDropdown
            editor={editor}
            isActive={state.isHighlight}
            currentColor={state.highlightColor}
            size="bubble"
          />
          <BubbleButton onClick={handleSetLink} isActive={state.isLink}>
            <LinkIcon className="h-3.5 w-3.5" />
          </BubbleButton>
        </div>
      </BubbleMenu>
      <div className="flex-1 overflow-auto pb-16 select-text">
        <EditorContent editor={editor} />
      </div>
    </div>
  );
}
