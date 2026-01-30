import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";
import { Rewind } from "lucide-react";

export function BackfillDropdown({
  availableSeconds,
  canCreate,
  onBackfill,
}: {
  availableSeconds: number;
  canCreate: boolean;
  onBackfill: (seconds?: number) => void;
}) {
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-xs"
              disabled={!canCreate}
            >
              <Rewind className="h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>
          Rewind ({availableSeconds}s available)
        </TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="start">
        <DropdownMenuItem onClick={() => onBackfill(availableSeconds)}>
          Full buffer ({availableSeconds}s)
        </DropdownMenuItem>
        {[30, 60, 120, 300].some((d) => d < availableSeconds) && (
          <DropdownMenuSeparator />
        )}
        {[30, 60, 120, 300]
          .filter((d) => d < availableSeconds)
          .map((d) => (
            <DropdownMenuItem key={d} onClick={() => onBackfill(d)}>
              Last {d}s
            </DropdownMenuItem>
          ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
