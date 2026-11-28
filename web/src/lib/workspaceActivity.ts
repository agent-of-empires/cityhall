import type { WorkspaceActivityUser } from "./api";

const STATE_LABEL: Record<Exclude<WorkspaceActivityUser["state"], "ok">, string> = {
  unreachable: "unreachable",
  auth_error: "auth error",
  not_running: "-",
};

/** A snapshot older than this (the poll runs every 10 seconds) is shown muted. */
export const STALE_AFTER_MS = 60_000;

/** What the last fetch of the activity endpoint gave. */
export interface ActivityFeed {
  users: Map<number, WorkspaceActivityUser>;
  sampledAt: string | null;
  /** The request failed, so nothing is known. */
  failed: boolean;
}

export const EMPTY_FEED: ActivityFeed = {
  users: new Map(),
  sampledAt: null,
  failed: false,
};

/** The text for the activity columns of one workspace. A user missing from the
 *  list has no fresh observation; it is not necessarily stopped. Counts only
 *  mean something when the state is `ok`. `stale` is true when the snapshot is
 *  old enough that the values may no longer hold. */
export function activityText(entry: WorkspaceActivityUser | undefined, feed: ActivityFeed, now = Date.now()) {
  const cells = (running: string) => ({
    running,
    waiting: "-",
    approvals: "-",
    lastAccess: "-",
    lastError: "-",
    stale: false,
  });
  if (feed.failed) return cells("unavailable");
  if (!feed.sampledAt) return cells("collecting");
  const stale = now - new Date(feed.sampledAt).getTime() > STALE_AFTER_MS;
  if (!entry || entry.state !== "ok") {
    return {
      ...cells(entry ? STATE_LABEL[entry.state as keyof typeof STATE_LABEL] : "-"),
      stale,
    };
  }
  return {
    running: String(entry.running_sessions),
    waiting: String(entry.waiting_sessions),
    approvals: String(entry.pending_approvals),
    lastAccess: entry.last_session_access ? new Date(entry.last_session_access).toLocaleString() : "-",
    lastError: entry.last_error ?? "-",
    stale,
  };
}
