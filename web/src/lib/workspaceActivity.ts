import type { WorkspaceActivityUser } from "./api";

const STATE_LABEL: Record<Exclude<WorkspaceActivityUser["state"], "ok">, string> = {
  unreachable: "unreachable",
  auth_error: "auth error",
  not_running: "-",
};

/** The text for the activity columns of one workspace. A user missing from the
 *  list is not running. Counts only mean something when the state is `ok`. */
export function activityText(entry: WorkspaceActivityUser | undefined) {
  if (!entry || entry.state !== "ok") {
    const label = entry ? STATE_LABEL[entry.state as keyof typeof STATE_LABEL] : "-";
    return { running: label, waiting: "-", approvals: "-", lastAccess: "-", lastError: "-" };
  }
  return {
    running: String(entry.running_sessions),
    waiting: String(entry.waiting_sessions),
    approvals: String(entry.pending_approvals),
    lastAccess: entry.last_session_access ? new Date(entry.last_session_access).toLocaleString() : "-",
    lastError: entry.last_error ?? "-",
  };
}
