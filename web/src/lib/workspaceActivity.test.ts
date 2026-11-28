import { describe, expect, it } from "vitest";
import type { WorkspaceActivityUser } from "./api";
import { activityText } from "./workspaceActivity";

const base: WorkspaceActivityUser = {
  user_id: 1,
  state: "ok",
  running_sessions: 2,
  waiting_sessions: 1,
  pending_approvals: 3,
  last_session_access: null,
  last_error: null,
};

describe("activityText", () => {
  it("shows counts when the state is ok", () => {
    expect(activityText({ ...base, last_error: "boom" })).toMatchObject({
      running: "2",
      waiting: "1",
      approvals: "3",
      lastAccess: "-",
      lastError: "boom",
    });
  });

  it("labels failures in the running column", () => {
    expect(activityText({ ...base, state: "auth_error" }).running).toBe("auth error");
    expect(activityText({ ...base, state: "unreachable" }).running).toBe("unreachable");
  });

  it("shows dashes for a user that is not running or absent", () => {
    expect(activityText({ ...base, state: "not_running" }).running).toBe("-");
    expect(activityText(undefined)).toEqual({
      running: "-",
      waiting: "-",
      approvals: "-",
      lastAccess: "-",
      lastError: "-",
    });
  });
});
