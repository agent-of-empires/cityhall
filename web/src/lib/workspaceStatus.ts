import type { StatusTone } from "../components/ui";
import type { WorkspaceItem, WorkspaceMode } from "./api";

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

/** Names for the two workspace modes (GLOSSARY.md), shared by the admin table
 *  and the member page. */
export const WORKSPACE_MODE_LABEL: Record<WorkspaceMode, string> = {
  full: "Full mode",
  composer: "Composer mode",
};

/** The mode label, plus the reason when the backend refused Full mode. */
export function modeText(mode: WorkspaceMode, note: string | null): string {
  return note ? `${WORKSPACE_MODE_LABEL[mode]} (${note})` : WORKSPACE_MODE_LABEL[mode];
}
