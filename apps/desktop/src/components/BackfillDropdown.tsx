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
import { BACKFILL_STEPS_SECONDS, formatBackfillSeconds } from "@/lib/backfill";

export function BackfillDropdown({
  availableSeconds,
  canCreate,
  onBackfill,
}: {
  availableSeconds: number;
  canCreate: boolean;
  onBackfill: (seconds?: number) => void;
}) {
  const visibleSteps = BACKFILL_STEPS_SECONDS.filter((d) => d < availableSeconds);
  const display = formatBackfillSeconds(availableSeconds);
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-xs"
              disabled={!canCreate || availableSeconds <= 0}
            >
              <Rewind className="h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>
          Rewind ({display} available)
        </TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="start">
        <DropdownMenuItem onClick={() => onBackfill(availableSeconds)}>
          Full buffer ({display})
        </DropdownMenuItem>
        {visibleSteps.length > 0 && <DropdownMenuSeparator />}
        {visibleSteps.map((d) => (
          <DropdownMenuItem key={d} onClick={() => onBackfill(d)}>
            Last {d}s
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
