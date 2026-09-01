import * as React from "react";
import { AppHuddleBar } from "@/app/AppHuddleBar";
import * as NiminoTheme from "@/app/NiminoThemeSurfaces";
import { HuddleProvider, useHuddle } from "@/features/huddle";
import { HUDDLE_SHORTCUT_EVENT } from "@/shared/lib/keyboard-shortcuts";
import { RemindMeLaterProvider } from "@/features/reminders/ui/RemindMeLaterProvider";
import { cn } from "@/shared/lib/cn";

type AppHuddleShellProps = {
  children: React.ReactNode;
  currentPubkey?: string;
  isCompanionOpen: boolean;
  isDrawerOpen: boolean;
  isRoom: boolean;
  onCompanionOpen: () => void;
  onHuddleStartPendingChange: (pending: boolean) => void;
  onHuddleStarted: (ephemeralChannelId: string) => void | Promise<void>;
  onShowHuddleInMainApp: (ephemeralChannelId: string) => void;
  onViewHuddleChannel: (ephemeralChannelId: string) => void;
  onVisibilityChange: (visible: boolean) => void;
};

type HuddleShortcutHandlerProps = {
  children: React.ReactNode;
};

function HuddleShortcutHandler({ children }: HuddleShortcutHandlerProps) {
  const { activeEphemeralChannelId, leaveHuddle } = useHuddle();

  React.useEffect(() => {
    if (!activeEphemeralChannelId) return;

    function handleHuddleShortcut() {
      void leaveHuddle();
    }

    window.addEventListener(HUDDLE_SHORTCUT_EVENT, handleHuddleShortcut);
    return () =>
      window.removeEventListener(HUDDLE_SHORTCUT_EVENT, handleHuddleShortcut);
  }, [activeEphemeralChannelId, leaveHuddle]);

  return children;
}

export function AppHuddleShell({
  children,
  currentPubkey,
  isCompanionOpen,
  isDrawerOpen,
  isRoom,
  onCompanionOpen,
  onHuddleStartPendingChange,
  onHuddleStarted,
  onShowHuddleInMainApp,
  onViewHuddleChannel,
  onVisibilityChange,
}: AppHuddleShellProps) {
  return (
    <HuddleProvider
      ownsAudioSession={!isRoom}
      onHuddleStartPendingChange={
        isRoom ? undefined : onHuddleStartPendingChange
      }
      onHuddleStarted={isRoom ? undefined : onHuddleStarted}
      onShowHuddleInMainApp={isRoom ? undefined : onShowHuddleInMainApp}
      onViewHuddleChannel={isRoom ? undefined : onViewHuddleChannel}
    >
      <HuddleShortcutHandler>
        <RemindMeLaterProvider pubkey={currentPubkey}>
          <div
            className="nimino-huddle-shell relative h-dvh overflow-hidden overscroll-none"
            data-huddle-open={isDrawerOpen}
            data-huddle-window={isRoom}
          >
            <div
              aria-hidden="true"
              className={cn(
                "nimino-huddle-drawer-backdrop",
                isDrawerOpen && "nimino-huddle-drawer-backdrop-open",
              )}
            />
            <div
              className={cn(
                "nimino-huddle-app-surface z-10 flex min-h-0 flex-row overflow-hidden bg-background",
                isDrawerOpen &&
                  (isRoom
                    ? "nimino-huddle-app-surface-room-open"
                    : "nimino-huddle-app-surface-open"),
              )}
            >
              <NiminoTheme.GradientLayer />
              {children}
            </div>
            {isRoom || !isCompanionOpen ? (
              <div className="nimino-huddle-drawer-slot absolute inset-x-0 bottom-0 z-[2] h-(--nimino-huddle-drawer-height)">
                <AppHuddleBar
                  mode={isRoom ? "room" : "main"}
                  onOpenHuddleWindow={isRoom ? undefined : onCompanionOpen}
                  onVisibilityChange={onVisibilityChange}
                />
              </div>
            ) : null}
          </div>
        </RemindMeLaterProvider>
      </HuddleShortcutHandler>
    </HuddleProvider>
  );
}
