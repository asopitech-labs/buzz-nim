import { isTauri } from "@tauri-apps/api/core";

import { clearSearchHitEventCache } from "@/app/navigation/searchHitEventCache";
import { resetActiveAgentTurnsStore } from "@/features/agents/activeAgentTurnsStore";
import { resetAgentWorkingSignal } from "@/features/agents/agentWorkingSignal";
import { resetCardMintStore } from "@/features/agents/cardMintStore";
import { resetAgentObserverStore } from "@/features/agents/observerRelayStore";
import { resetPendingOpenCreateAgent } from "@/features/agents/openCreateAgentEvent";
import { resetPendingOpenEditAgent } from "@/features/agents/openEditAgentEvent";
import { resetPendingSnapshotImport } from "@/features/agents/openSnapshotImportFromUrlEvent";
import { resetWelcomeComposerBannerState } from "@/features/channels/ui/useWelcomeComposerBanner";
import { resetBackgroundMediaUploads } from "@/features/messages/lib/backgroundMediaUploadStore";
import { resetLinkPreviewPreparations } from "@/features/messages/lib/linkPreviewPreparationStore";
import { resetPersistentAgentAudienceStore } from "@/features/messages/lib/persistentAgentAudience";
import { resetRenderScopedReactionHydration } from "@/features/messages/lib/renderScopedReactions";
import { clearAllDrafts } from "@/features/messages/lib/useDrafts";
import { clearTimeoutState } from "@/features/moderation/lib/timeoutStore";
import { resetAvatarPresentations } from "@/features/profile/avatarPresentationStore";
import { resetAvatarProfileSync } from "@/features/profile/avatarProfileSync";
import { resetSidebarRelayConnectionCardState } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { resetTerminalPanel } from "@/features/terminal/terminalPanelStore";
import { relayClient } from "@/shared/api/relayClient";
import { resetRateLimitGate } from "@/shared/api/relayRateLimitGate";
import { clearTrayAgentActivity } from "@/shared/api/trayMenu";
import { resetNavigationDeepLinkDrain } from "@/shared/deep-link";
import { resetMediaCaches } from "@/shared/lib/mediaUrl";
import { isMacPlatform } from "@/shared/lib/platform";
import { resetLinkPreviewMetadataCache } from "@/shared/lib/useResolvedLinkPreviews";
import { clearMarkdownNodeCache } from "@/shared/ui/markdown/nodeCache";
import { resetMessageLinkMetadataCache } from "@/shared/ui/markdown/useMessageLinkMetadata";
import { resetVideoPlayerState } from "@/shared/ui/videoPlayerState";

/** Canonical owner for every community-scoped module singleton. */
export async function resetCommunityState({
  resetAvatarState,
}: {
  resetAvatarState: boolean;
}): Promise<void> {
  relayClient.disconnect();
  await resetNavigationDeepLinkDrain();
  resetRateLimitGate();
  clearAllDrafts();
  clearTimeoutState();
  resetTerminalPanel();
  resetAgentObserverStore();
  resetActiveAgentTurnsStore();
  resetAgentWorkingSignal();
  resetCardMintStore();
  resetPendingOpenCreateAgent();
  resetPendingOpenEditAgent();
  resetPendingSnapshotImport();
  resetWelcomeComposerBannerState();
  if (isTauri() && isMacPlatform()) void clearTrayAgentActivity();
  if (resetAvatarState) {
    resetAvatarProfileSync();
    resetAvatarPresentations();
  }
  resetSidebarRelayConnectionCardState();
  resetMediaCaches();
  resetLinkPreviewMetadataCache();
  resetVideoPlayerState();
  resetRenderScopedReactionHydration();
  resetBackgroundMediaUploads();
  resetLinkPreviewPreparations();
  resetPersistentAgentAudienceStore();
  clearSearchHitEventCache();
  clearMarkdownNodeCache();
  resetMessageLinkMetadataCache();
}
