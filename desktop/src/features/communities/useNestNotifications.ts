import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { toast } from "sonner";

/**
 * Surface repository-directory backend errors as toasts.
 *
 * - `repos-dir-error`: a configured `repos_dir` failed to validate or its
 *   symlink could not be applied (invalid path, downgrade refused, external
 *   target gone). Emitted by `apply_workspace` on both the validate-reject
 *   and the runtime symlink-failure paths, so a bad `repos_dir` is always
 *   visibly surfaced rather than silently logged to console.
 * Mounted at the app root ahead of the community-init effect so the listener
 * is registered before the first `apply_workspace` call.
 */
export function useNestNotifications(): void {
  useEffect(() => {
    const unlistenReposError = listen<string>("repos-dir-error", (event) => {
      toast.error("Repos directory not applied", {
        description: event.payload,
      });
    });

    return () => {
      void unlistenReposError.then((fn) => fn());
    };
  }, []);
}
