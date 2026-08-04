import { useCallback, useEffect, useRef, useState } from "react";
import GridLayout, { useContainerWidth, type Layout } from "react-grid-layout";
import { Check, Pencil, RotateCcw } from "lucide-react";
import { api, ApiError, type Dashboard, type DashboardLayout, type DashboardLayoutItem, type Me } from "../lib/api";
import {
  applyPositions,
  defaultLayout,
  EDIT_MIN_WIDTH,
  formatAge,
  GRID_COLUMNS,
  mergeLayout,
  toggleWidget,
  visibleWidgets,
  WIDGETS,
} from "../lib/dashboardLayout";
import { TopBar } from "./TopBar";
import { Button } from "./ui";
import { WIDGET_BODIES } from "./dashboard/widgets";

/// Matched to the sampler's own interval: polling faster only re-reads the same
/// snapshot, since the request path does no collection of its own.
const POLL_MS = 10_000;
const ROW_HEIGHT = 40;

export function DashboardPage({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
  const [data, setData] = useState<Dashboard | null>(null);
  const [layout, setLayout] = useState<DashboardLayout>(() => defaultLayout());
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Re-rendered on a timer so the freshness line counts up between polls
  // rather than sitting on a stale "10s ago".
  const [, setTick] = useState(0);
  const { width, containerRef, mounted } = useContainerWidth();
  // Positions from the current drag session, kept out of state so a poll
  // cannot re-render mid-drag and fight the pointer. Only the positions live
  // here; visibility stays in `layout`, so a session that just toggles a
  // widget still saves.
  const dragged = useRef<DashboardLayoutItem[] | null>(null);

  const load = useCallback(async () => {
    try {
      setData(await api.dashboard());
      setError(null);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "could not load the dashboard");
    }
  }, []);

  useEffect(() => {
    void load();
    api
      .getDashboardLayout()
      .then(({ layout: stored }) => setLayout(mergeLayout(stored)))
      // A layout that will not load is not worth blocking the dashboard for;
      // the defaults are a working fallback.
      .catch(() => setLayout(defaultLayout()));
  }, [load]);

  // Paused while editing: a re-render mid-drag fights the pointer.
  useEffect(() => {
    if (editing) return;
    const refresh = () => {
      if (!document.hidden) void load();
    };
    const poll = setInterval(refresh, POLL_MS);
    const age = setInterval(() => setTick((t) => t + 1), 1_000);
    document.addEventListener("visibilitychange", refresh);
    window.addEventListener("focus", refresh);
    return () => {
      clearInterval(poll);
      clearInterval(age);
      document.removeEventListener("visibilitychange", refresh);
      window.removeEventListener("focus", refresh);
    };
  }, [editing, load]);

  const canEdit = width >= EDIT_MIN_WIDTH;
  // Leaving a narrow viewport mid-edit would otherwise strand the Done button.
  useEffect(() => {
    if (!canEdit) setEditing(false);
  }, [canEdit]);

  async function save(next: DashboardLayout) {
    setLayout(next);
    try {
      await api.saveDashboardLayout(next);
      setError(null);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "could not save the layout");
    }
  }

  function onLayoutChange(next: Layout) {
    if (!editing) return;
    dragged.current = next.map((item) => ({ id: item.i, x: item.x, y: item.y, w: item.w, h: item.h }));
  }

  async function done() {
    setEditing(false);
    const positions = dragged.current;
    dragged.current = null;
    // Saved from live state either way: an edit session that only toggled a
    // widget off never produces a drag, and would otherwise save nothing.
    await save(positions ? applyPositions(layout, positions) : layout);
  }

  async function reset() {
    // Drop any positions from this session, or Done would fold them back in
    // on top of the defaults just restored.
    dragged.current = null;
    await save(defaultLayout());
  }

  const visible = visibleWidgets(layout);
  const gridLayout: Layout = visible.map((widget) => {
    const item = layout.items.find((i) => i.id === widget.id) ?? { ...widget.default, id: widget.id };
    return { i: widget.id, x: item.x, y: item.y, w: item.w, h: item.h, minW: widget.minW, minH: widget.minH };
  });

  return (
    <div className="flex h-full flex-col">
      <TopBar me={me} onLogout={onLogout} />
      <main className="flex-1 overflow-auto p-6">
        <div className="mx-auto w-full max-w-7xl space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex items-baseline gap-3">
              <h2 className="font-mono text-xs uppercase tracking-wider text-text-muted">Dashboard</h2>
              {data && (
                <span className={data.stale ? "text-xs text-status-error" : "text-xs text-text-muted"}>
                  {data.sampled_at ? `updated ${formatAge(data.sampled_at)}` : "collecting the first sample"}
                </span>
              )}
            </div>
            {canEdit && (
              <div className="flex items-center gap-2">
                {editing && (
                  <Button variant="ghost" onClick={() => void reset()} className="flex items-center gap-1.5">
                    <RotateCcw size={14} />
                    Reset layout
                  </Button>
                )}
                <Button
                  variant={editing ? "primary" : "ghost"}
                  onClick={() => (editing ? void done() : setEditing(true))}
                  className="flex items-center gap-1.5"
                >
                  {editing ? <Check size={14} /> : <Pencil size={14} />}
                  {editing ? "Done" : "Edit layout"}
                </Button>
              </div>
            )}
          </div>

          {error && <p className="text-sm text-status-error">{error}</p>}

          {data && data.errors.length > 0 && (
            <div className="space-y-1 rounded-md border border-surface-700 bg-surface-850 px-4 py-3 text-sm">
              {data.errors.map((message) => (
                <p key={message} className="text-status-waiting">
                  {message}
                </p>
              ))}
              <p className="text-xs text-text-muted">The figures below are the last ones collected successfully.</p>
            </div>
          )}

          {editing && (
            <div className="flex flex-wrap items-center gap-3 rounded-md border border-surface-700 bg-surface-850 px-4 py-3">
              <span className="text-sm text-text-secondary">Drag a card by its title; resize from its corner.</span>
              {WIDGETS.map((widget) => (
                <label key={widget.id} className="flex items-center gap-1.5 text-xs text-text-secondary">
                  <input
                    type="checkbox"
                    checked={!layout.hidden.includes(widget.id)}
                    onChange={() => setLayout((current) => toggleWidget(current, widget.id))}
                    className="h-4 w-4 accent-brand-500"
                  />
                  {widget.title}
                </label>
              ))}
            </div>
          )}

          <div ref={containerRef}>
            {mounted && data && (
              <GridLayout
                width={width}
                layout={gridLayout}
                onLayoutChange={onLayoutChange}
                gridConfig={{ cols: GRID_COLUMNS, rowHeight: ROW_HEIGHT, margin: [16, 16], containerPadding: [0, 0] }}
                // Off until Edit, and handled by the title bar only, so a click
                // on a table row or a button never starts a drag.
                dragConfig={{ enabled: editing, handle: ".dashboard-drag-handle" }}
                resizeConfig={{ enabled: editing }}
              >
                {visible.map((widget) => {
                  const Body = WIDGET_BODIES[widget.id];
                  return (
                    <div
                      key={widget.id}
                      className="flex flex-col overflow-hidden rounded-lg border border-surface-700 bg-surface-900"
                    >
                      <div
                        className={`dashboard-drag-handle border-b border-surface-700 px-4 py-2 ${editing ? "cursor-move" : ""}`}
                      >
                        <h3 className="font-mono text-xs uppercase tracking-wider text-text-muted">{widget.title}</h3>
                      </div>
                      <div className="flex-1 overflow-auto p-4">{Body && <Body data={data} />}</div>
                    </div>
                  );
                })}
              </GridLayout>
            )}
            {!data && !error && <p className="text-sm text-text-muted">Loading...</p>}
          </div>
        </div>
      </main>
    </div>
  );
}
