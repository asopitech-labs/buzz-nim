import * as React from "react";

import { isWelcomeChannel } from "@/features/onboarding/welcome";
import type { Channel } from "@/shared/api/types";

/** `done` is terminal so a resolved empty timeline cannot restart setup. */
export type WelcomeKickoffStagePhase = "hidden" | "active" | "done";

/**
 * How long the stage waits for the first agent message before settling into
 * the quiet timed-out state. Generous because the teammate presence wait
 * alone can take up to 60s (see welcomeKickoff.ts TEAMMATE_READY_WAIT_MS).
 */
export const WELCOME_KICKOFF_STAGE_TIMEOUT_MS = 90_000;

export type WelcomeKickoffStageInput = {
  /** The active channel is the private Welcome channel. */
  isWelcome: boolean;
  /** The timeline query has settled — an empty list means truly empty. */
  timelineSettled: boolean;
  /** Any message exists in the channel (agent or user authored). */
  hasMessages: boolean;
  /** The timeout window elapsed while the stage was active. */
  timedOut: boolean;
};

/** Pure status transition; product state never waits for visual motion. */
export function resolveWelcomeKickoffStagePhase(
  current: WelcomeKickoffStagePhase,
  input: WelcomeKickoffStageInput,
): WelcomeKickoffStagePhase {
  // Checked before `isWelcome` so the terminal state can never be laundered
  // back into `hidden` (and from there into a replay) by a channel that
  // momentarily reads as non-Welcome. Real channel changes reset the hook.
  if (current === "done") return "done";
  if (!input.isWelcome) return "hidden";
  if (current === "hidden") {
    return input.timelineSettled && !input.hasMessages ? "active" : "hidden";
  }
  if (input.hasMessages || input.timedOut) return "done";
  return current;
}

/**
 * Whether the banner copy may claim the team is still being set up. True only
 * while that is actually happening.
 */
export function isWelcomeKickoffSettingUp(phase: WelcomeKickoffStagePhase) {
  return phase === "active";
}

/**
 * Drives the Welcome kickoff status from local state only — no network
 * round-trips or animation lifecycle.
 *
 * `hasTimelineMessages` must reflect *visible timeline rows* (the formatted
 * message list), not raw channel events. A fresh Welcome channel already
 * carries non-message events (canvas seed, membership records) that render
 * nothing — gating on raw events keeps the stage hidden forever.
 */
export function useWelcomeKickoffStage(
  activeChannel: Channel | null,
  hasTimelineMessages: boolean,
  timelineLoading: boolean,
) {
  const channelId = activeChannel?.id ?? null;
  const isWelcome = isWelcomeChannel(activeChannel);
  const [phase, setPhase] = React.useState<WelcomeKickoffStagePhase>("hidden");
  const [timedOut, setTimedOut] = React.useState(false);

  // biome-ignore lint/correctness/useExhaustiveDependencies: reset stage state exactly when the active channel changes.
  React.useEffect(() => {
    setPhase("hidden");
    setTimedOut(false);
  }, [channelId]);

  React.useEffect(() => {
    setPhase((current) =>
      resolveWelcomeKickoffStagePhase(current, {
        isWelcome,
        timelineSettled: !timelineLoading,
        hasMessages: hasTimelineMessages,
        timedOut,
      }),
    );
  }, [hasTimelineMessages, isWelcome, timedOut, timelineLoading]);

  React.useEffect(() => {
    if (phase !== "active") return;
    const timer = globalThis.setTimeout(
      () => setTimedOut(true),
      WELCOME_KICKOFF_STAGE_TIMEOUT_MS,
    );
    return () => globalThis.clearTimeout(timer);
  }, [phase]);

  return { phase };
}
