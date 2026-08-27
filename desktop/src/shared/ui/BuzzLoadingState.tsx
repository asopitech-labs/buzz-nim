import { cn } from "@/shared/lib/cn";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";

/** Centered, low-emphasis loading state for page and panel fetches. */
export function BuzzLoadingState({
  className,
  fill = false,
  label = "Loading",
}: {
  className?: string;
  fill?: boolean;
  label?: string;
}) {
  return (
    <div
      className={cn(
        "flex w-full items-center justify-center text-muted-foreground/45",
        fill ? "min-h-0 flex-1" : "min-h-[calc(100dvh-7rem)]",
        className,
      )}
      data-testid="buzz-loading-state"
      role="status"
    >
      <span aria-hidden="true" className="w-8">
        <BuzzMark className="h-auto w-full" />
      </span>
      <span className="sr-only">{label}</span>
    </div>
  );
}
