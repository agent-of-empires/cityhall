import { Fragment, useCallback, useEffect, useState } from "react";
import { ExternalLink, KeyRound, Play, Square, Trash2 } from "lucide-react";
import { api, ApiError, can, type Me, type WorkspaceItem } from "../lib/api";
import { isOlderVersion } from "../lib/versions";
import { AgentCredentialsEditor } from "./AgentCredentials";
import { TopBar } from "./TopBar";
import { Button, Select } from "./ui";
import { VersionField } from "./VersionField";

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
  const canImpersonate = can(me, "workspaces.impersonate");
  const [items, setItems] = useState<WorkspaceItem[]>([]);
  const [proxyOrigin, setProxyOrigin] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [version, setVersion] = useState("");
  const [restart, setRestart] = useState(false);
  const [busy, setBusy] = useState(false);
  const [versions, setVersions] = useState<string[]>([]);
  const [latest, setLatest] = useState<string | null>(null);
  const [expandedUserId, setExpandedUserId] = useState<number | null>(null);

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
    api
      .listWorkspaceVersions()
      .then((v) => {
        setVersions(v.versions);
        setLatest(v.latest);
      })
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

  // Paired up so the banner can read the message without re-narrowing it.
  const provisioning = items.flatMap((item) => (item.provisioning ? [{ item, info: item.provisioning }] : []));

  // Statuses change outside this tab (the proxy auto-starts workspaces on
  // access), so refresh when the tab regains focus and poll slowly while
  // visible.
  useEffect(() => {
    const refresh = () => {
      if (!document.hidden) void load();
    };
    document.addEventListener("visibilitychange", refresh);
    window.addEventListener("focus", refresh);
    const timer = setInterval(refresh, 10_000);
    return () => {
      document.removeEventListener("visibilitychange", refresh);
      window.removeEventListener("focus", refresh);
      clearInterval(timer);
    };
  }, [load]);

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
    await run(() => api.bulkSetWorkspaceVersion(ids, version.trim() || null, restart), "could not set version");
    setSelected(new Set());
    setVersion("");
  }

  function outdated(item: WorkspaceItem): boolean {
    // A hand-built tag (e.g. "main-20260804") is not a release, so comparing
    // it numerically against `latest` would parse garbage version components.
    // Only flag outdated when the version is one of the discovered releases.
    return (
      latest !== null &&
      item.effective_version !== null &&
      versions.includes(item.effective_version) &&
      isOlderVersion(item.effective_version, latest)
    );
  }

  function selectOutdated() {
    setSelected(new Set(items.filter(outdated).map((i) => i.user_id)));
  }

  async function openAsAdmin(item: WorkspaceItem) {
    if (
      !confirm(
        `Open ${item.username}'s workspace? Every workspace tab in this browser will show ` +
          `their workspace until you exit (via "Open workspace" in the top bar) or 30 minutes pass. ` +
          `This access is audited.`,
      )
    ) {
      return;
    }
    await run(async () => {
      const { url } = await api.workspaceAccessUrl(item.user_id);
      window.open(url, "_blank", "noopener");
    }, "could not open workspace");
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
          {canWrite && items.some(outdated) && (
            <Button variant="ghost" disabled={busy} onClick={selectOutdated}>
              Select outdated
            </Button>
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

        {provisioning.length > 0 && (
          <div className="space-y-1.5 rounded-md border border-surface-700 bg-surface-850 px-4 py-3 text-sm">
            {provisioning.map(({ item, info }) => (
              <p key={item.user_id} className={info.failed ? "text-status-error" : "text-status-waiting"}>
                <span className="font-medium text-text-primary">{item.username}</span>: {info.message}
              </p>
            ))}
            <p className="text-xs text-text-muted">
              A first image pull or local build takes a few minutes and continues if this page is closed.
            </p>
          </div>
        )}

        {canWrite && selected.size > 0 && (
          <div className="flex flex-wrap items-center gap-3 rounded-md border border-surface-700 bg-surface-850 px-4 py-3">
            <span className="text-sm text-text-secondary">{selected.size} selected</span>
            <VersionField
              value={version}
              onChange={setVersion}
              versions={versions}
              latest={latest ?? undefined}
              noneLabel="follow default"
              className="w-52"
            />
            <label className="flex items-center gap-1.5 text-xs text-text-secondary">
              <input
                type="checkbox"
                checked={restart}
                onChange={(e) => setRestart(e.target.checked)}
                className="h-4 w-4 accent-brand-500"
              />
              restart running now
            </label>
            <Button variant="primary" disabled={busy} onClick={applyVersion}>
              Set version
            </Button>
            <Button variant="ghost" onClick={() => setSelected(new Set())}>
              Clear
            </Button>
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
                <Fragment key={item.user_id}>
                  <tr className="border-b border-surface-800 last:border-0">
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
                        </span>
                      ) : (
                        <span className={STATUS_STYLES[item.status]}>{STATUS_LABELS[item.status]}</span>
                      )}
                    </td>
                    <td className="px-4 py-2.5 text-text-secondary">
                      <div className="flex items-center gap-2">
                        {canWrite && versions.length > 0 ? (
                          <Select
                            value={item.pinned_version ?? ""}
                            disabled={busy}
                            onChange={(e) =>
                              run(
                                () => api.setWorkspaceVersion(item.user_id, e.target.value || null, restart),
                                "could not set version",
                              )
                            }
                            className="w-44"
                          >
                            <option value="">
                              {item.effective_version ? `default (${item.effective_version})` : "default"}
                            </option>
                            {/* A previously saved version can predate the discovered list. */}
                            {item.pinned_version && !versions.includes(item.pinned_version) && (
                              <option value={item.pinned_version} title={item.pinned_version}>
                                {item.pinned_version}
                              </option>
                            )}
                            {versions.map((v) => (
                              <option key={v} value={v}>
                                {v}
                                {v === latest ? " (latest)" : ""}
                              </option>
                            ))}
                          </Select>
                        ) : (
                          <span>
                            {item.pinned_version
                              ? item.pinned_version
                              : item.effective_version
                                ? `default (${item.effective_version})`
                                : "-"}
                          </span>
                        )}
                        {outdated(item) && (
                          <span className="text-xs font-medium text-status-waiting" title={`latest is ${latest}`}>
                            outdated
                          </span>
                        )}
                      </div>
                    </td>
                    <td className="px-4 py-2.5 text-text-secondary">
                      {item.last_active_at ? new Date(item.last_active_at).toLocaleString() : "-"}
                    </td>
                    {canWrite && (
                      <td className="px-4 py-2.5">
                        <div className="flex justify-end gap-1">
                          {canImpersonate && item.user_id !== me.id && (
                            <Button
                              variant="ghost"
                              disabled={busy}
                              onClick={() => openAsAdmin(item)}
                              title="Open this user's workspace (audited)"
                            >
                              <ExternalLink size={14} />
                            </Button>
                          )}
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
                          <Button
                            variant="ghost"
                            className={item.user_id === expandedUserId ? "text-brand-500" : undefined}
                            onClick={() => setExpandedUserId(item.user_id === expandedUserId ? null : item.user_id)}
                            title="Agent credentials"
                          >
                            <KeyRound size={14} />
                          </Button>
                        </div>
                      </td>
                    )}
                  </tr>
                  {canWrite && item.user_id === expandedUserId && (
                    <tr className="border-b border-surface-800 last:border-0">
                      <td colSpan={6} className="bg-surface-850 px-4 py-4">
                        {/* Status is already visible in the row above: an admin seeding or
                          rotating a credential can see whether that user's workspace is
                          currently running before using the existing start/stop controls
                          to apply it. */}
                        <AgentCredentialsEditor
                          load={() => api.getUserAgentCredentials(item.user_id)}
                          update={(envVar, value) => api.updateUserAgentCredential(item.user_id, envVar, value)}
                          remove={(envVar) => api.deleteUserAgentCredential(item.user_id, envVar)}
                        />
                      </td>
                    </tr>
                  )}
                </Fragment>
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
