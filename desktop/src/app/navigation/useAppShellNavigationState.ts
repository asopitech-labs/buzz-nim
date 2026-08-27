import * as React from "react";

import { deriveShellRoute } from "@/app/AppShell.helpers";
import { useBackForwardControls } from "@/app/navigation/useBackForwardControls";

export function useAppShellNavigationState(pathname: string, href: string) {
  const [isNewMessageOpen, setIsNewMessageOpen] = React.useState(false);
  const shellRoute = React.useMemo(
    () => deriveShellRoute(pathname),
    [pathname],
  );
  const closeNewMessage = React.useCallback(
    () => setIsNewMessageOpen(false),
    [],
  );
  const openNewMessage = React.useCallback(() => setIsNewMessageOpen(true), []);
  const history = useBackForwardControls(
    isNewMessageOpen ? closeNewMessage : undefined,
  );
  const previousHrefRef = React.useRef(href);

  React.useEffect(() => {
    if (previousHrefRef.current === href) return;
    previousHrefRef.current = href;
    closeNewMessage();
  }, [closeNewMessage, href]);

  return {
    ...history,
    closeNewMessage,
    isNewMessageOpen,
    openNewMessage,
    selectedChannelId: isNewMessageOpen ? null : shellRoute.selectedChannelId,
    selectedView: isNewMessageOpen ? "messages" : shellRoute.selectedView,
  };
}
