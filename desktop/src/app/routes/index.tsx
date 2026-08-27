import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { HomeScreen } from "@/features/home/ui/HomeScreen";
import {
  consumePendingWelcomeChannel,
  WELCOME_CHANNEL_READY_EVENT,
} from "@/features/onboarding/welcome";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useFeatureEnabled } from "@/shared/features";
import { Button } from "@/shared/ui/button";

const PulseScreen = React.lazy(async () => {
  const module = await import("@/features/pulse/ui/PulseScreen");
  return { default: module.PulseScreen };
});

type HomeRouteSearch = {
  item?: string;
  profile?: string;
  profileTab?: string;
  profileView?: string;
  view?: "activity";
};

function validateHomeSearch(search: Record<string, unknown>): HomeRouteSearch {
  return {
    item:
      typeof search.item === "string" && search.item.length > 0
        ? search.item
        : undefined,
    profile:
      typeof search.profile === "string" && search.profile.length > 0
        ? search.profile
        : undefined,
    profileTab:
      typeof search.profileTab === "string" && search.profileTab.length > 0
        ? search.profileTab
        : undefined,
    profileView:
      typeof search.profileView === "string" && search.profileView.length > 0
        ? search.profileView
        : undefined,
    view: search.view === "activity" ? "activity" : undefined,
  };
}

export const Route = createFileRoute("/")({
  validateSearch: validateHomeSearch,
  component: HomeRouteComponent,
});

function HomeRouteComponent() {
  const { goActivity, goChannel, goHome } = useAppNavigation();
  const { view } = Route.useSearch();
  const activityEnabled = useFeatureEnabled("pulse");
  const showActivity = activityEnabled && view === "activity";
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const channels = channelsQuery.data ?? [];
  const availableChannelIds = React.useMemo(
    () => new Set(channels.map((channel) => channel.id)),
    [channels],
  );
  const availableChannelIdsRef = React.useRef(availableChannelIds);
  const openPendingWelcomeChannel = React.useCallback(
    (ids: ReadonlySet<string>) => {
      const welcomeChannelId = consumePendingWelcomeChannel(ids);
      if (!welcomeChannelId) {
        return;
      }

      void goChannel(welcomeChannelId, { replace: true });
    },
    [goChannel],
  );

  React.useEffect(() => {
    availableChannelIdsRef.current = availableChannelIds;
  }, [availableChannelIds]);

  React.useEffect(() => {
    function handleWelcomeChannelReady() {
      openPendingWelcomeChannel(availableChannelIdsRef.current);
    }

    window.addEventListener(
      WELCOME_CHANNEL_READY_EVENT,
      handleWelcomeChannelReady,
    );
    return () => {
      window.removeEventListener(
        WELCOME_CHANNEL_READY_EVENT,
        handleWelcomeChannelReady,
      );
    };
  }, [openPendingWelcomeChannel]);

  React.useEffect(() => {
    openPendingWelcomeChannel(availableChannelIds);
  }, [availableChannelIds, openPendingWelcomeChannel]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div
        aria-label="Home sections"
        className="flex shrink-0 items-center justify-center gap-1 border-b border-border/35 bg-background px-4 py-2"
        data-testid="home-surface-tabs"
        role="tablist"
      >
        <Button
          aria-selected={!showActivity}
          data-testid="home-inbox-tab"
          onClick={() => void goHome()}
          role="tab"
          size="sm"
          type="button"
          variant={showActivity ? "ghost" : "secondary"}
        >
          Inbox
        </Button>
        {activityEnabled ? (
          <Button
            aria-selected={showActivity}
            data-testid="home-activity-tab"
            onClick={() => void goActivity()}
            role="tab"
            size="sm"
            type="button"
            variant={showActivity ? "secondary" : "ghost"}
          >
            Activity
          </Button>
        ) : null}
      </div>
      {showActivity ? (
        <React.Suspense fallback={null}>
          <PulseScreen />
        </React.Suspense>
      ) : (
        <HomeScreen
          availableChannelIds={availableChannelIds}
          currentPubkey={identityQuery.data?.pubkey}
          onOpenContext={(channelId, messageId, threadRootId) => {
            void goChannel(channelId, { messageId, threadRootId });
          }}
        />
      )}
    </div>
  );
}
