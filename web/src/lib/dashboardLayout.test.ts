import { describe, expect, it } from "vitest";
import type { DashboardLayout } from "./api";
import {
  applyPositions,
  defaultLayout,
  formatAge,
  formatBytes,
  formatPercent,
  GRID_COLUMNS,
  LAYOUT_SCHEMA_VERSION,
  mergeLayout,
  toggleWidget,
  visibleWidgets,
  WIDGETS,
} from "./dashboardLayout";

// There is no @testing-library/react in this repo, so these cover the pure
// layout reconciliation and formatting the dashboard is built on, which is
// where the drift between a saved layout and the shipped widget catalog is
// actually handled.

describe("mergeLayout", () => {
  it("falls back to defaults for anything unusable", () => {
    expect(mergeLayout(null)).toEqual(defaultLayout());
    // A layout written by a future build must not half-apply.
    expect(mergeLayout({ schema_version: 99, items: [], hidden: [] })).toEqual(defaultLayout());
    // Hand-edited junk that got past a stale validator.
    expect(mergeLayout({ schema_version: 1, items: null, hidden: null } as unknown as DashboardLayout)).toEqual(
      defaultLayout(),
    );
  });

  it("keeps saved positions", () => {
    const stored: DashboardLayout = {
      schema_version: LAYOUT_SCHEMA_VERSION,
      items: WIDGETS.map((w, i) => ({ id: w.id, x: 0, y: i * 4, w: 12, h: 4 })),
      hidden: [],
    };
    const merged = mergeLayout(stored);
    expect(merged.items).toHaveLength(WIDGETS.length);
    expect(merged.items[0]).toEqual({ id: WIDGETS[0].id, x: 0, y: 0, w: 12, h: 4 });
  });

  it("appends a widget added after the layout was saved", () => {
    // A layout from a release that only had the first widget.
    const stored: DashboardLayout = {
      schema_version: LAYOUT_SCHEMA_VERSION,
      items: [{ id: WIDGETS[0].id, x: 0, y: 0, w: 6, h: 4 }],
      hidden: [],
    };
    const merged = mergeLayout(stored);
    expect(merged.items).toHaveLength(WIDGETS.length);
    // Placed below what was already positioned rather than on top of it.
    const appended = merged.items.filter((i) => i.id !== WIDGETS[0].id);
    expect(appended.every((i) => i.y >= 4)).toBe(true);
  });

  it("drops ids for widgets that no longer exist", () => {
    const stored: DashboardLayout = {
      schema_version: LAYOUT_SCHEMA_VERSION,
      items: [
        { id: WIDGETS[0].id, x: 0, y: 0, w: 6, h: 4 },
        { id: "widget-from-an-old-release", x: 6, y: 0, w: 6, h: 4 },
      ],
      hidden: ["also-gone"],
    };
    const merged = mergeLayout(stored);
    expect(merged.items.map((i) => i.id).sort()).toEqual(WIDGETS.map((w) => w.id).sort());
    expect(merged.hidden).toEqual([]);
  });

  it("honors hidden widgets that still exist", () => {
    const stored: DashboardLayout = { ...defaultLayout(), hidden: [WIDGETS[1].id] };
    const merged = mergeLayout(stored);
    expect(merged.hidden).toEqual([WIDGETS[1].id]);
    expect(visibleWidgets(merged).map((w) => w.id)).not.toContain(WIDGETS[1].id);
    expect(visibleWidgets(merged)).toHaveLength(WIDGETS.length - 1);
  });

  it("clamps geometry into the grid", () => {
    const stored: DashboardLayout = {
      schema_version: LAYOUT_SCHEMA_VERSION,
      items: [
        // Wider than the grid, off the right edge, and negative.
        { id: WIDGETS[0].id, x: 99, y: -5, w: 99, h: 0 },
        { id: WIDGETS[1].id, x: -1, y: 0, w: 1, h: 1 },
      ],
      hidden: [],
    };
    const merged = mergeLayout(stored);
    for (const item of merged.items) {
      expect(item.x).toBeGreaterThanOrEqual(0);
      expect(item.y).toBeGreaterThanOrEqual(0);
      expect(item.w).toBeGreaterThan(0);
      expect(item.h).toBeGreaterThan(0);
      expect(item.x + item.w).toBeLessThanOrEqual(GRID_COLUMNS);
    }
    // A widget narrower than its minimum is widened, not left unusable.
    const second = merged.items.find((i) => i.id === WIDGETS[1].id)!;
    expect(second.w).toBeGreaterThanOrEqual(WIDGETS[1].minW);
    expect(second.h).toBeGreaterThanOrEqual(WIDGETS[1].minH);
  });

  it("produces a layout the backend will accept", () => {
    const merged = mergeLayout(null);
    expect(merged.schema_version).toBe(LAYOUT_SCHEMA_VERSION);
    expect(merged.items.length).toBeLessThanOrEqual(32);
    const ids = merged.items.map((i) => i.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).toMatch(/^[a-z][a-z0-9-]{0,63}$/);
  });
});

describe("applyPositions", () => {
  it("keeps the stored position of a widget the grid did not render", () => {
    // The grid only reports what it drew, so a hidden widget is absent from
    // the callback. Overwriting items wholesale would lose where it sat, which
    // is the position it needs when it is shown again.
    const start = { ...defaultLayout(), hidden: [WIDGETS[1].id] };
    const fromGrid = [{ id: WIDGETS[0].id, x: 6, y: 2, w: 6, h: 5 }];

    const applied = applyPositions(start, fromGrid);
    expect(applied.items.find((i) => i.id === WIDGETS[0].id)).toEqual(fromGrid[0]);
    expect(applied.items.find((i) => i.id === WIDGETS[1].id)).toEqual(start.items.find((i) => i.id === WIDGETS[1].id));
    // Every widget survives, and visibility is untouched.
    expect(applied.items).toHaveLength(WIDGETS.length);
    expect(applied.hidden).toEqual([WIDGETS[1].id]);
  });

  it("is a no-op when nothing moved", () => {
    const start = defaultLayout();
    expect(applyPositions(start, [])).toEqual(start);
  });
});

describe("toggleWidget", () => {
  it("hides and re-shows without losing position", () => {
    const start = defaultLayout();
    const hiddenOnce = toggleWidget(start, WIDGETS[0].id);
    expect(hiddenOnce.hidden).toEqual([WIDGETS[0].id]);
    // Position survives being hidden, so re-showing puts it back where it was.
    expect(hiddenOnce.items).toEqual(start.items);
    expect(toggleWidget(hiddenOnce, WIDGETS[0].id).hidden).toEqual([]);
  });
});

describe("formatBytes", () => {
  it("scales into IEC units", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1024)).toBe("1.0 KiB");
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(1024 * 1024)).toBe("1.0 MiB");
    expect(formatBytes(1024 ** 3 * 7.6)).toBe("7.6 GiB");
    // Past 10 the decimal is noise on an instantaneous sample.
    expect(formatBytes(1024 ** 3 * 34)).toBe("34 GiB");
  });

  it("renders absent values as a dash rather than zero", () => {
    expect(formatBytes(null)).toBe("-");
    expect(formatBytes(undefined)).toBe("-");
    expect(formatBytes(NaN)).toBe("-");
    expect(formatBytes(-1)).toBe("-");
  });
});

describe("formatPercent", () => {
  it("keeps a multi-core figure above 100", () => {
    // A container on two cores legitimately reads 200%; clamping would hide
    // the most interesting row on the page.
    expect(formatPercent(250)).toBe("250%");
    expect(formatPercent(0)).toBe("0.0%");
    expect(formatPercent(7.5)).toBe("7.5%");
    // Above 10 the decimal is dropped.
    expect(formatPercent(72.8)).toBe("73%");
    expect(formatPercent(null)).toBe("-");
  });
});

describe("formatAge", () => {
  it("describes snapshot freshness", () => {
    const now = Date.parse("2026-08-04T12:00:30Z");
    expect(formatAge(null, now)).toBe("collecting");
    expect(formatAge("2026-08-04T12:00:30Z", now)).toBe("just now");
    expect(formatAge("2026-08-04T12:00:20Z", now)).toBe("10s ago");
    expect(formatAge("2026-08-04T11:58:30Z", now)).toBe("2m ago");
    expect(formatAge("2026-08-04T09:00:30Z", now)).toBe("3h ago");
    expect(formatAge("not a date", now)).toBe("collecting");
  });
});
