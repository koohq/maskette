import { useQuery } from "@tanstack/react-query";
import { useSyncExternalStore } from "react";
import { inDesktopWindow } from "../../api/backend";
import { readStored, writeStored } from "../../app/storage";
import { checkForUpdate, getUpdateStatus } from "./updateApi";

export const CHECK_ON_LAUNCH_STORAGE_KEY = "maskette.updateCheckOnLaunch.v1";

/** Whether the app looks for a new version at startup. On unless the user turned it off in the settings. */
export function readCheckOnLaunch(): boolean {
  return readStored(CHECK_ON_LAUNCH_STORAGE_KEY) !== "0";
}

export function writeCheckOnLaunch(enabled: boolean): void {
  writeStored(CHECK_ON_LAUNCH_STORAGE_KEY, enabled ? "1" : "0");
}

export const UPDATE_CHECK_KEY = ["update", "check"] as const;
export const UPDATE_STATUS_KEY = ["update", "status"] as const;

/**
 * The check for a new version: made once per launch when the user has not turned it off, and again whenever the
 * settings page's "Check now" refetches it. Both share this cache, so a version found either way shows on the list.
 * Never in a plain browser (the dev server), where there is no installed app to update.
 */
export function useUpdateCheck() {
  return useQuery({
    queryKey: UPDATE_CHECK_KEY,
    queryFn: checkForUpdate,
    enabled: inDesktopWindow() && readCheckOnLaunch(),
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: Number.POSITIVE_INFINITY,
    retry: false,
    refetchOnWindowFocus: false,
  });
}

/** Whether updates are supported and how an install is going; polled while one runs. */
export function useUpdateStatus() {
  return useQuery({
    queryKey: UPDATE_STATUS_KEY,
    queryFn: getUpdateStatus,
    enabled: inDesktopWindow(),
    retry: false,
    refetchOnWindowFocus: false,
    refetchInterval: (query) => {
      const phase = query.state.data?.install.phase;
      return phase === "downloading" || phase === "installing" ? 500 : false;
    },
  });
}

// The version whose banner the user closed. Kept for this launch only: the next launch shows it again.
let dismissedVersion: string | null = null;
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useDismissedUpdate(): string | null {
  return useSyncExternalStore(subscribe, () => dismissedVersion);
}

export function dismissUpdate(version: string): void {
  dismissedVersion = version;
  for (const listener of listeners) listener();
}

/** For tests: forget the closed banner, as a new launch would. */
export function resetDismissedUpdate(): void {
  dismissedVersion = null;
  for (const listener of listeners) listener();
}
