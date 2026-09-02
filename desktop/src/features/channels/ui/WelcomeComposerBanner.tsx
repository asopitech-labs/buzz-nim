import type * as React from "react";
import { motion } from "motion/react";
import { Bot, Check, X } from "lucide-react";

import { ComposerDockGlassBackdrop } from "@/features/messages/ui/ComposerDockBackdrop";
import { cn } from "@/shared/lib/cn";

const WELCOME_PERSONA_NAME = "Fizz";
export const WELCOME_COMPOSER_BANNER_SUCCESS_DISPLAY_MS = 3200;
const WELCOME_COMPOSER_BANNER_EASE = [0.22, 1, 0.36, 1] as const;
export const WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS = 0.18;
export const WELCOME_COMPOSER_BANNER_HIDE_BUFFER_MS = 50;
const WELCOME_COMPOSER_BANNER_DISMISS_Y_OFFSET_PX = 48;

export type WelcomeComposerBannerState =
  | "prompt"
  | "complete"
  | "dismissing"
  | "hidden";

export function containsWelcomePersonaMention(content: string) {
  return content
    .toLowerCase()
    .includes(`@${WELCOME_PERSONA_NAME.toLowerCase()}`);
}

function WelcomeComposerPersonaMention() {
  return (
    <span
      className="inline-block whitespace-nowrap align-baseline font-medium leading-[inherit] text-foreground"
      data-active-persona={WELCOME_PERSONA_NAME}
      data-persona-options={WELCOME_PERSONA_NAME}
      data-testid="welcome-composer-persona-mention"
    >
      @{WELCOME_PERSONA_NAME}
    </span>
  );
}

type WelcomeComposerBannerProps = {
  state: WelcomeComposerBannerState;
  /**
   * While the Welcome kickoff is still setting up the team, the banner's
   * prompt copy reads as a setup status ("Setting up your welcome team…")
   * instead of the mention hint.
   */
  settingUp?: boolean;
  /**
   * Called when the user dismisses the banner manually via the close button.
   * Only rendered while `state === "prompt"`.
   */
  onDismiss?: () => void;
};

export function WelcomeComposerBanner({
  onDismiss,
  settingUp = false,
  state,
}: WelcomeComposerBannerProps) {
  if (state === "hidden") {
    return null;
  }

  return (
    <motion.div
      animate={{
        height: state === "dismissing" ? 0 : "auto",
      }}
      className="overflow-hidden"
      initial={false}
      transition={{
        duration:
          state === "dismissing"
            ? WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS
            : 0,
        ease: WELCOME_COMPOSER_BANNER_EASE,
      }}
    >
      <motion.div
        animate={{
          y:
            state === "dismissing"
              ? WELCOME_COMPOSER_BANNER_DISMISS_Y_OFFSET_PX
              : 0,
        }}
        className={cn(
          "relative z-[1] mx-5 mb-0 flex items-center gap-2 rounded-t-2xl border border-b-0 px-4 pb-5 pt-2.5 text-sm leading-5 transition-colors",
          state !== "prompt"
            ? "border-emerald-500/30 bg-emerald-500/15 text-foreground"
            : "border-border/60 bg-muted/55 text-muted-foreground",
        )}
        data-state={state}
        data-testid="welcome-composer-guide-banner"
        data-tone={state !== "prompt" ? "success" : "neutral"}
        initial={false}
        transition={{
          duration:
            state === "dismissing"
              ? WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS
              : 0,
          ease: WELCOME_COMPOSER_BANNER_EASE,
        }}
      >
        {state !== "prompt" ? (
          <>
            <span className="flex h-4 w-4 shrink-0 items-center justify-center text-foreground">
              <Check
                aria-hidden
                className="h-4 w-4"
                data-testid="welcome-composer-complete-icon"
              />
            </span>
            <span className="min-w-0 flex-1">Nice work.</span>
          </>
        ) : (
          <>
            <span className="flex h-4 w-4 shrink-0 items-center justify-center text-muted-foreground">
              <Bot aria-hidden className="h-4 w-4" />
            </span>
            {settingUp ? (
              <span
                className="min-w-0 flex-1"
                data-testid="welcome-composer-setting-up-copy"
              >
                Setting up your welcome team…
              </span>
            ) : (
              <span className="min-w-0 flex-1">
                Mention <WelcomeComposerPersonaMention /> or another teammate
                whenever you want their help.
              </span>
            )}
          </>
        )}
        {state === "prompt" && onDismiss && !settingUp ? (
          <button
            aria-label="Dismiss hint"
            className="ml-auto flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-muted-foreground/60 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="welcome-composer-dismiss-button"
            onClick={onDismiss}
            type="button"
          >
            <X aria-hidden className="h-3 w-3" />
          </button>
        ) : null}
      </motion.div>
    </motion.div>
  );
}

type WelcomeComposerGuidanceLayerProps = WelcomeComposerBannerProps & {
  children: React.ReactNode;
};

export function WelcomeComposerGuidanceLayer({
  children,
  onDismiss,
  settingUp,
  state,
}: WelcomeComposerGuidanceLayerProps) {
  if (state === "hidden") {
    return null;
  }

  return (
    <div className="relative" data-testid="welcome-composer-guidance-layer">
      <ComposerDockGlassBackdrop
        className="absolute inset-x-5 top-0 bottom-3 z-0 rounded-t-2xl"
        testId="welcome-composer-guidance-backdrop"
      />
      {children}
      <WelcomeComposerBanner
        onDismiss={onDismiss}
        settingUp={settingUp}
        state={state}
      />
    </div>
  );
}
