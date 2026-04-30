export const BACKFILL_STEPS_SECONDS = [5, 10, 15, 30, 60, 120, 300] as const;

export function formatBackfillSeconds(s: number): string {
  if (s <= 0) return "0s";
  if (s < 10) return `${s.toFixed(1)}s`;
  return `${Math.round(s)}s`;
}
