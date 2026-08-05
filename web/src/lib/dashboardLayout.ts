import type { DashboardLayout, DashboardLayoutItem } from "./api";

/// The layout shape the backend accepts. Bumped only on a breaking change; a
/// stored layout at any other version is discarded for the defaults.
export const LAYOUT_SCHEMA_VERSION = 1;
/// Grid width every widget position is expressed in.
export const GRID_COLUMNS = 12;
/// Below this the grid is too narrow to arrange sensibly, so editing is hidden
/// rather than letting someone save a layout mangled by a phone viewport.
export const EDIT_MIN_WIDTH = 768;

export interface WidgetDefinition {
  id: string;
  title: string;
  /// Where this widget sits on a dashboard nobody has customized.
  default: Omit<DashboardLayoutItem, "id">;
  minW: number;
  minH: number;
}

/// Every widget the dashboard can show, in the order a fresh dashboard stacks
/// them. Ids are the stored identity, so renaming one drops that widget from
/// every saved layout; add a new id instead.
///
/// Default heights are sized to hold each widget's content at ROW_HEIGHT
/// without an inner scrollbar, since a card that is born scrolling reads as
/// broken rather than as resizable.
export const WIDGETS: WidgetDefinition[] = [
  {
    id: "system-usage",
    title: "System usage",
    default: { x: 0, y: 0, w: 6, h: 7 },
    minW: 3,
    minH: 4,
  },
  {
    id: "fleet-status",
    title: "Workspace status",
    default: { x: 6, y: 0, w: 6, h: 7 },
    minW: 3,
    minH: 4,
  },
  {
    id: "containers",
    title: "Containers",
    default: { x: 0, y: 7, w: 12, h: 6 },
    minW: 4,
    minH: 4,
  },
  {
    id: "versions",
    title: "Versions",
    default: { x: 0, y: 13, w: 6, h: 4 },
    minW: 3,
    minH: 3,
  },
  {
    id: "provisioning",
    title: "Provisioning",
    default: { x: 6, y: 13, w: 6, h: 4 },
    minW: 3,
    minH: 3,
  },
];

export function defaultLayout(): DashboardLayout {
  return {
    schema_version: LAYOUT_SCHEMA_VERSION,
    items: WIDGETS.map((w) => ({ id: w.id, ...w.default })),
    hidden: [],
  };
}

/// Clamp one stored rectangle into the grid.
///
/// A saved layout can predate a widget's current minimum size, and nothing
/// stops a hand-edited row holding absurd coordinates, so geometry is fixed up
/// here rather than trusted. The backend bounds what it stores; this bounds
/// what actually gets rendered.
function clamp(item: DashboardLayoutItem, widget: WidgetDefinition): DashboardLayoutItem {
  const w = Math.min(GRID_COLUMNS, Math.max(widget.minW, Math.floor(item.w) || widget.default.w));
  const h = Math.max(widget.minH, Math.floor(item.h) || widget.default.h);
  const x = Math.min(GRID_COLUMNS - w, Math.max(0, Math.floor(item.x) || 0));
  const y = Math.max(0, Math.floor(item.y) || 0);
  return { id: item.id, x, y, w, h };
}

/// Reconcile a stored layout with the widget catalog this build ships.
///
/// Three things drift between a saved layout and the code reading it: a widget
/// was removed, a widget was added, or the stored payload is junk. All three
/// have to resolve to something renderable, because the alternative is a
/// dashboard that cannot load until someone clears a database row.
export function mergeLayout(stored: DashboardLayout | null): DashboardLayout {
  if (!stored || stored.schema_version !== LAYOUT_SCHEMA_VERSION || !Array.isArray(stored.items)) {
    return defaultLayout();
  }

  const hidden = (Array.isArray(stored.hidden) ? stored.hidden : []).filter((id) => WIDGETS.some((w) => w.id === id));

  // Widgets keep their saved position; ones added in a later release are
  // appended below everything rather than overlapping what is already placed.
  let nextY = stored.items.reduce(
    (max, item) => Math.max(max, (Math.floor(item.y) || 0) + (Math.floor(item.h) || 1)),
    0,
  );
  const items = WIDGETS.map((widget) => {
    const saved = stored.items.find((item) => item?.id === widget.id);
    if (!saved) {
      const appended = { id: widget.id, ...widget.default, y: nextY };
      nextY += widget.default.h;
      return appended;
    }
    return clamp(saved, widget);
  });

  return { schema_version: LAYOUT_SCHEMA_VERSION, items, hidden };
}

/// Fold the grid's reported positions back into a full layout.
///
/// The grid only ever reports the widgets it rendered, so a hidden widget is
/// absent from `positions`. Overwriting `items` with that list wholesale would
/// throw away where a hidden widget used to sit, which is the position it needs
/// when it is shown again.
export function applyPositions(layout: DashboardLayout, positions: DashboardLayoutItem[]): DashboardLayout {
  const moved = new Map(positions.map((p) => [p.id, p]));
  return { ...layout, items: layout.items.map((item) => moved.get(item.id) ?? item) };
}

/// The widgets to render, in layout order, skipping hidden ones.
export function visibleWidgets(layout: DashboardLayout): WidgetDefinition[] {
  return WIDGETS.filter((w) => !layout.hidden.includes(w.id));
}

export function toggleWidget(layout: DashboardLayout, id: string): DashboardLayout {
  const hidden = layout.hidden.includes(id) ? layout.hidden.filter((h) => h !== id) : [...layout.hidden, id];
  return { ...layout, hidden };
}

/// Byte counts as an operator reads them. IEC units, because that is what
/// container runtimes report and mixing the two on one page invites the reader
/// to compare figures that are 7% apart for no visible reason.
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes) || bytes < 0) return "-";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const units = ["KiB", "MiB", "GiB", "TiB", "PiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // One decimal below 10 keeps "1.5 GiB" readable without implying precision
  // an instantaneous sample does not have.
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export function formatPercent(percent: number | null | undefined): string {
  if (percent === null || percent === undefined || !Number.isFinite(percent)) return "-";
  return `${percent < 10 ? percent.toFixed(1) : Math.round(percent)}%`;
}

/// How long ago the snapshot was taken, for the freshness line. Sampled data is
/// near-real-time, and saying so is the difference between "the dashboard is
/// wrong" and "the dashboard is ten seconds behind".
export function formatAge(sampledAt: string | null, now: number = Date.now()): string {
  if (!sampledAt) return "collecting";
  const seconds = Math.round((now - new Date(sampledAt).getTime()) / 1000);
  if (!Number.isFinite(seconds)) return "collecting";
  if (seconds < 1) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.round(minutes / 60)}h ago`;
}
