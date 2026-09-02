import { cn } from "@/shared/lib/cn";
import { NiminoMark } from "@/shared/ui/nimino-logo/NiminoMark";

export function TurnLivenessIndicator({ className }: { className?: string }) {
  return (
    <div
      aria-label="Agent turn in progress"
      className={cn("w-5 opacity-25", className)}
      data-testid="turn-liveness-indicator"
      role="status"
    >
      <NiminoMark className="w-full text-foreground" />
    </div>
  );
}
