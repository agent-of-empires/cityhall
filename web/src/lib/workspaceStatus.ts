import type { StatusTone } from "../components/ui";
import type { WorkspaceItem } from "./api";

/** One status vocabulary for every surface that shows a workspace, so the
 *  dashboard table and the workspaces table cannot drift apart. Glyphs match
 *  the ones the aoe TUI uses. */
export const WORKSPACE_STATUS_META: Record<
  WorkspaceItem["status"],
  { tone: StatusTone; glyph: string; label: string }
> = {
  running: { tone: "running", glyph: "●", label: "running" },
  stopped: { tone: "idle", glyph: "■", label: "stopped" },
  not_created: { tone: "idle", glyph: "○", label: "not created" },
  unknown: { tone: "error", glyph: "?", label: "unknown" },
};
