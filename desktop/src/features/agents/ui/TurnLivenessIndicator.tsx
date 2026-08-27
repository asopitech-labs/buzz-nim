import { cn } from "@/shared/lib/cn";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";

export function TurnLivenessIndicator({ className }: { className?: string }) {
  return (
    <div
      aria-label="Agent turn in progress"
      className={cn("w-5 opacity-25", className)}
      data-testid="turn-liveness-indicator"
      role="status"
    >
      <BuzzMark className="w-full text-foreground" />
    </div>
  );
}
