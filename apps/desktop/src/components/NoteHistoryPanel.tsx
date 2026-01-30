import { useState, useEffect } from "react";
import { getNote, getNoteVersions } from "@/lib/db";
import type { DbNoteVersion } from "@/lib/db";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { History } from "lucide-react";

export function NoteHistoryPanel({
  sessionId,
  onRestore,
}: {
  sessionId: string;
  onRestore?: (content: string) => void;
}) {
  const [versions, setVersions] = useState<DbNoteVersion[]>([]);
  const [isOpen, setIsOpen] = useState(false);

  useEffect(() => {
    if (!isOpen) return;
    getNote(sessionId).then((note) => {
      if (note) {
        getNoteVersions(note.id).then(setVersions);
      }
    });
  }, [sessionId, isOpen]);

  return (
    <Popover open={isOpen} onOpenChange={setIsOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              variant="ghost"
              size="icon-xs"
              className="text-muted-foreground"
            >
              <History />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent side="bottom">Version history</TooltipContent>
      </Tooltip>
      <PopoverContent align="end" className="w-64 p-0">
        <div className="px-3 py-2 border-b">
          <p className="text-xs font-medium">Version history</p>
        </div>
        <ScrollArea className="max-h-[200px]">
          <div className="space-y-1 p-2">
            {versions.length === 0 && (
              <p className="text-xs text-muted-foreground py-2 text-center">
                No versions yet
              </p>
            )}
            {versions.map((version) => (
              <div
                key={version.id}
                className="flex items-center justify-between rounded-md px-2 py-1.5 text-xs hover:bg-muted/50"
              >
                <span className="text-muted-foreground">
                  {new Date(version.created_at + "Z").toLocaleString()}
                </span>
                {onRestore && (
                  <Button
                    variant="ghost"
                    size="xs"
                    onClick={() => onRestore(version.content)}
                  >
                    Restore
                  </Button>
                )}
              </div>
            ))}
          </div>
        </ScrollArea>
      </PopoverContent>
    </Popover>
  );
}
