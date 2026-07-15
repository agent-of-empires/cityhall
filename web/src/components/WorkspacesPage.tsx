import { useCallback, useEffect, useState } from "react";
import { Play, Square, Trash2 } from "lucide-react";
import { api, ApiError, can, type Me, type WorkspaceItem } from "../lib/api";
import { TopBar } from "./TopBar";
import { Button, Input } from "./ui";

const STATUS_STYLES: Record<WorkspaceItem["status"], string> = {
  running: "text-status-running",
  stopped: "text-status-waiting",
  not_created: "text-text-muted",
  unknown: "text-status-error",
};

const STATUS_LABELS: Record<WorkspaceItem["status"], string> = {
  running: "running",
  stopped: "stopped",
  not_created: "not created",
  unknown: "unknown",
};

export function WorkspacesPage({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
  const canWrite = can(me, "workspaces.write");
  const [items, setItems] = useState<WorkspaceItem[]>([]);
  const [proxyOrigin, setProxyOrigin] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setItems(await api.listWorkspaces());
      setError(null);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "could not load workspaces");
    }
  }, []);

  useEffect(() => {
    void load();
    api
      .myWorkspace()
      .then((w) => setProxyOrigin(w.proxy_origin))
      .catch(() => {});
  }, [load]);

  // While an image pull/build or binary download runs, poll so progress and
  // completion show up without a manual refresh.
  const anyProvisioning = items.some((i) => i.provisioning && !i.provisioning.failed);
  useEffect(() => {
    if (!anyProvisioning) return;
    const timer = setInterval(() => void load(), 3000);
    return () => clearInterval(timer);
  }, [anyProvisioning, load]);

  function toggle(userId: number) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(userId)) next.delete(userId);
      else next.add(userId);
      return next;
    });
  }

  async function run(action: () => Promise<unknown>, failure: string) {
    setBusy(true);
    try {
      await action();
      setError(null);
      await load();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : failure);
    } finally {
      setBusy(false);
    }
  }

  async function applyVersion() {
    const ids = [...selected];
    await run(() => api.bulkSetWorkspaceVersion(ids, version.trim() || null), "could not set version");
    setSelected(new Set());
    setVersion("");
  }

  async function destroy(item: WorkspaceItem) {
    if (!confirm(`Destroy ${item.username}'s workspace? This permanently deletes its data volume.`)) {
      return;
    }
    await run(() => api.destroyWorkspace(item.user_id), "could not destroy workspace");
  }

  return (
    <div className="flex h-full flex-col">
      <TopBar me={me} onLogout={onLogout} />
      <main className="mx-auto w-full max-w-4xl flex-1 space-y-4 overflow-auto p-6">
        <div className="flex items-center justify-between">
          <h2 className="font-mono text-xs uppercase tracking-wider text-text-muted">Workspaces</h2>
          {canWrite && (
            <div className="flex items-end gap-2">
              <Input
                value={version}
                onChange={(e) => setVersion(e.target.value)}
                placeholder="version, empty = default"
                className="w-52"
              />
              <Button variant="primary" disabled={busy || selected.size === 0} onClick={applyVersion}>
                Set version ({selected.size})
              </Button>
            </div>
          )}
        </div>

        {error && <p className="text-sm text-status-error">{error}</p>}

        {proxyOrigin && (
          <div className="rounded-md border border-surface-700 bg-surface-850 px-4 py-3 text-sm text-text-secondary">
            All workspaces are served at{" "}
            <a href={proxyOrigin} target="_blank" rel="noreferrer" className="text-text-primary underline">
              {proxyOrigin}
            </a>
            ; each signed-in user reaches their own workspace there. Containers listen on internal loopback ports
            managed automatically.
          </div>
        )}

        <div className="overflow-hidden rounded-lg border border-surface-700">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-surface-700 bg-surface-850 text-left font-mono text-xs uppercase tracking-wider text-text-muted">
                {canWrite && <th className="w-8 px-4 py-2.5" />}
                <th className="px-4 py-2.5 font-medium">User</th>
                <th className="px-4 py-2.5 font-medium">Status</th>
                <th className="px-4 py-2.5 font-medium">Version</th>
                <th className="px-4 py-2.5 font-medium">Last active</th>
                {canWrite && <th className="px-4 py-2.5 text-right font-medium">Actions</th>}
              </tr>
            </thead>
            <tbody>
              {items.map((item) => (
                <tr key={item.user_id} className="border-b border-surface-800 last:border-0">
                  {canWrite && (
                    <td className="px-4 py-2.5">
                      <input
                        type="checkbox"
                        checked={selected.has(item.user_id)}
                        onChange={() => toggle(item.user_id)}
                        className="h-4 w-4 accent-brand-500"
                      />
                    </td>
                  )}
                  <td className="px-4 py-2.5 text-text-primary">{item.username}</td>
                  <td className="px-4 py-2.5">
                    {item.provisioning ? (
                      <span
                        className={item.provisioning.failed ? "text-status-error" : "text-status-waiting"}
                        title={item.provisioning.message}
                      >
                        {item.provisioning.failed ? "provisioning failed" : "provisioning"}
                        <span className="block max-w-56 truncate text-xs text-text-muted">
                          {item.provisioning.message}
                        </span>
                      </span>
                    ) : (
                      <span className={STATUS_STYLES[item.status]}>{STATUS_LABELS[item.status]}</span>
                    )}
                  </td>
                  <td className="px-4 py-2.5 text-text-secondary">
                    {item.pinned_version ?? (item.effective_version ? `default (${item.effective_version})` : "-")}
                  </td>
                  <td className="px-4 py-2.5 text-text-secondary">
                    {item.last_active_at ? new Date(item.last_active_at).toLocaleString() : "-"}
                  </td>
                  {canWrite && (
                    <td className="px-4 py-2.5">
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          disabled={busy || item.status === "running"}
                          onClick={() => run(() => api.startWorkspace(item.user_id), "could not start workspace")}
                          title="Start"
                        >
                          <Play size={14} />
                        </Button>
                        <Button
                          variant="ghost"
                          disabled={busy || item.status !== "running"}
                          onClick={() => run(() => api.stopWorkspace(item.user_id), "could not stop workspace")}
                          title="Stop (keeps data)"
                        >
                          <Square size={14} />
                        </Button>
                        <Button
                          variant="ghost"
                          disabled={busy || item.status === "not_created"}
                          onClick={() => destroy(item)}
                          title="Destroy (deletes data)"
                        >
                          <Trash2 size={14} />
                        </Button>
                      </div>
                    </td>
                  )}
                </tr>
              ))}
              {items.length === 0 && (
                <tr>
                  <td colSpan={canWrite ? 6 : 4} className="px-4 py-6 text-center text-text-muted">
                    No users.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </main>
    </div>
  );
}
