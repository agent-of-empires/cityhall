import { describe, expect, it } from "vitest";
import type { WorkspaceActivityUser } from "./api";
import { activityText, EMPTY_FEED, type ActivityFeed } from "./workspaceActivity";

const base: WorkspaceActivityUser = {
  user_id: 1,
  state: "ok",
  running_sessions: 2,
  waiting_sessions: 1,
  pending_approvals: 3,
  last_session_access: null,
  last_error: null,
};

const NOW = Date.parse("2026-07-15T09:21:00Z");
const fresh: ActivityFeed = {
  users: new Map(),
  sampledAt: "2026-07-15T09:20:55Z",
  failed: false,
};

describe("activityText", () => {
  it("shows counts when the state is ok", () => {
    expect(activityText({ ...base, last_error: "boom" }, fresh, NOW)).toMatchObject({
      running: "2",
      waiting: "1",
      approvals: "3",
      lastAccess: "-",
      lastError: "boom",
    });
  });

  it("labels failures in the running column", () => {
    expect(activityText({ ...base, state: "auth_error" }, fresh, NOW).running).toBe("auth error");
    expect(activityText({ ...base, state: "unreachable" }, fresh, NOW).running).toBe("unreachable");
  });

  it("shows dashes for a user that is not running or absent", () => {
    expect(activityText({ ...base, state: "not_running" }, fresh, NOW).running).toBe("-");
    expect(activityText(undefined, fresh, NOW)).toEqual({
      running: "-",
      waiting: "-",
      approvals: "-",
      lastAccess: "-",
      lastError: "-",
      stale: false,
    });
  });

  it("says unavailable when the request failed, whatever the entry", () => {
    const failed = { ...EMPTY_FEED, failed: true };
    expect(activityText(undefined, failed, NOW).running).toBe("unavailable");
    expect(activityText(base, failed, NOW).running).toBe("unavailable");
  });

  it("says collecting before the first sample", () => {
    expect(activityText(undefined, EMPTY_FEED, NOW).running).toBe("collecting");
  });

  it("marks a snapshot older than a minute as stale and keeps its values", () => {
    const old = { ...fresh, sampledAt: "2026-07-15T09:19:00Z" };
    expect(activityText(base, old, NOW)).toMatchObject({
      running: "2",
      stale: true,
    });
    expect(activityText(base, fresh, NOW).stale).toBe(false);
  });
});
