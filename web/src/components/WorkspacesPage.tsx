import clsx from "clsx";
import { Fragment, useCallback, useEffect, useState, type ReactNode } from "react";
import { ExternalLink } from "lucide-react";
import { Link } from "react-router-dom";
import { api, ApiError, can, type Me, type WorkspaceItem } from "../lib/api";
import { isOlderVersion } from "../lib/versions";
import { PageBody, PageHeader } from "./AppShell";
import { AgentCredentialsEditor } from "./AgentCredentials";
import { FirstRunModal } from "./FirstRunModal";
import {
  Banner,
  Button,
  Card,
  Checkbox,
  ErrorText,
  SectionLabel,
  Select,
  StatusText,
  tableCardClass,
  tableHeadClass,
  tdClass,
  thClass,
  trClass,
} from "./ui";
import { VersionField } from "./VersionField";
import { WORKSPACE_STATUS_META as STATUS_META } from "../lib/workspaceStatus";

// Dismissing onboarding only persists on the server; `me` is fetched once by
// App and not refreshed here, so a real dismissal would otherwise look
// undone again after navigating away and back within the same session. Keyed by
// user id because logging out does not reload the page, and the next user in
// this tab has their own flag on the server.
const onboardingDismissedInSession = new Set<number>();

export function WorkspacesPage({ me }: { me: Me }) {
  const canRead = can(me, "workspaces.read");
  const canUse = can(me, "workspaces.use");
  if (canRead) return <AdminWorkspaces me={me} />;
  if (canUse) return <MemberWorkspace me={me} />;
  return (
    <>
      <PageHeader title="Workspaces" />
      <PageBody>
        <Banner>Your role does not include access to a workspace. Ask an administrator for it.</Banner>
      </PageBody>
    </>
  );
}

function AdminWorkspaces({ me }: { me: Me }) {
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

  async function bulkStart() {
    const ids = items.filter((i) => selected.has(i.user_id) && i.status !== "running").map((i) => i.user_id);
    await run(() => Promise.all(ids.map((id) => api.startWorkspace(id))), "could not start the selected workspaces");
    setSelected(new Set());
  }

  async function bulkStop() {
    const ids = items.filter((i) => selected.has(i.user_id) && i.status === "running").map((i) => i.user_id);
    await run(() => Promise.all(ids.map((id) => api.stopWorkspace(id))), "could not stop the selected workspaces");
    setSelected(new Set());
  }

  async function bulkDestroy() {
    const targets = items.filter((i) => selected.has(i.user_id) && i.status !== "not_created");
    if (targets.length === 0) return;
    if (
      !confirm(
        `Destroy ${targets.length} selected workspace${targets.length === 1 ? "" : "s"}? This permanently deletes ` +
          `their data volumes.`,
      )
    ) {
      return;
    }
    await run(
      () => Promise.all(targets.map((i) => api.destroyWorkspace(i.user_id))),
      "could not destroy the selected workspaces",
    );
    setSelected(new Set());
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
          `their workspace until you exit (via "Open workspace" in the sidebar) or 30 minutes pass. ` +
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
    <>
      <PageHeader
        title="Workspaces"
        meta="docker backend"
        actions={
          canWrite && items.some(outdated) ? (
            <Button variant="default" disabled={busy} onClick={selectOutdated}>
              Select outdated
            </Button>
          ) : undefined
        }
      />
      <PageBody className="flex flex-col gap-3.5">
        {error && <ErrorText>{error}</ErrorText>}

        {proxyOrigin && (
          <Banner>
            All workspaces are served at{" "}
            <a href={proxyOrigin} target="_blank" rel="noreferrer" className="text-text underline">
              {proxyOrigin}
            </a>
            ; each signed-in user reaches their own workspace there. A workspace starts on the first request and stops
            after the idle timeout, keeping its volume.
          </Banner>
        )}

        {provisioning.length > 0 && (
          <Banner className="space-y-1.5">
            {provisioning.map(({ item, info }) => (
              <p key={item.user_id} className={info.failed ? "text-error" : "text-waiting"}>
                <span className="font-medium text-text-bright">{item.username}</span>: {info.message}
              </p>
            ))}
            <p className="text-xs text-text-hint">
              A first image pull or local build takes a few minutes and continues if this page is closed.
            </p>
          </Banner>
        )}

        <div className={tableCardClass}>
          {canWrite && selected.size > 0 && (
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border-soft bg-accent-soft px-[18px] py-2.5">
              <span className="font-mono text-xs text-accent-bright">{selected.size} selected</span>
              <div className="flex flex-wrap items-center gap-2">
                <Button disabled={busy} onClick={() => void bulkStart()}>
                  Start
                </Button>
                <Button disabled={busy} onClick={() => void bulkStop()}>
                  Stop
                </Button>
                <VersionField
                  value={version}
                  onChange={setVersion}
                  versions={versions}
                  latest={latest ?? undefined}
                  noneLabel="follow default"
                  className="w-52"
                />
                <Checkbox checked={restart} onChange={setRestart} label="restart running now" />
                <Button variant="primary" disabled={busy} onClick={() => void applyVersion()}>
                  Pin version
                </Button>
                <Button variant="danger" disabled={busy} onClick={() => void bulkDestroy()}>
                  Destroy
                </Button>
                <Button variant="ghost" onClick={() => setSelected(new Set())}>
                  Clear
                </Button>
              </div>
            </div>
          )}

          <table className="w-full text-sm">
            <thead className={tableHeadClass}>
              <tr>
                {canWrite && <th className={clsx(thClass, "w-9")} />}
                <th className={thClass}>User</th>
                <th className={thClass}>Status</th>
                <th className={thClass}>Version</th>
                <th className={thClass}>Last active</th>
                {canWrite && <th className={clsx(thClass, "text-right")}>Actions</th>}
              </tr>
            </thead>
            <tbody>
              {items.map((item) => {
                const meta = STATUS_META[item.status];
                return (
                  <Fragment key={item.user_id}>
                    <tr className={trClass}>
                      {canWrite && (
                        <td className={tdClass}>
                          <Checkbox checked={selected.has(item.user_id)} onChange={() => toggle(item.user_id)} />
                        </td>
                      )}
                      <td className={clsx(tdClass, "text-text-bright")}>{item.username}</td>
                      <td className={tdClass}>
                        {item.provisioning ? (
                          <span title={item.provisioning.message}>
                            <StatusText
                              tone={item.provisioning.failed ? "error" : "waiting"}
                              glyph={item.provisioning.failed ? "✕" : "◐"}
                            >
                              {item.provisioning.failed ? "provisioning failed" : "provisioning"}
                            </StatusText>
                          </span>
                        ) : (
                          <StatusText tone={meta.tone} glyph={meta.glyph}>
                            {meta.label}
                          </StatusText>
                        )}
                      </td>
                      <td className={tdClass}>
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
                            <span className="text-text-dim">
                              {item.pinned_version
                                ? item.pinned_version
                                : item.effective_version
                                  ? `default (${item.effective_version})`
                                  : "-"}
                            </span>
                          )}
                          {outdated(item) && (
                            <span className="font-mono text-xs text-waiting" title={`latest is ${latest}`}>
                              outdated
                            </span>
                          )}
                        </div>
                      </td>
                      <td className={clsx(tdClass, "font-mono text-text-dim")}>
                        {item.last_active_at ? new Date(item.last_active_at).toLocaleString() : "-"}
                      </td>
                      {canWrite && (
                        <td className={tdClass}>
                          <div className="flex justify-end gap-3.5">
                            {canImpersonate && item.user_id !== me.id && (
                              <RowAction onClick={() => openAsAdmin(item)} disabled={busy} title="Open (audited)">
                                open
                              </RowAction>
                            )}
                            <RowAction
                              onClick={() => run(() => api.startWorkspace(item.user_id), "could not start workspace")}
                              disabled={busy || item.status === "running"}
                              title="Start"
                            >
                              start
                            </RowAction>
                            <RowAction
                              onClick={() => run(() => api.stopWorkspace(item.user_id), "could not stop workspace")}
                              disabled={busy || item.status !== "running"}
                              title="Stop (keeps data)"
                            >
                              stop
                            </RowAction>
                            <RowAction
                              onClick={() => setExpandedUserId(item.user_id === expandedUserId ? null : item.user_id)}
                              tone={item.user_id === expandedUserId ? "active" : "faint"}
                              title="Set this user's agent credentials"
                            >
                              keys
                            </RowAction>
                            <RowAction
                              onClick={() => destroy(item)}
                              disabled={busy || item.status === "not_created"}
                              tone="danger"
                              title="Destroy (deletes data)"
                            >
                              ✕
                            </RowAction>
                          </div>
                        </td>
                      )}
                    </tr>
                    {canWrite && item.user_id === expandedUserId && (
                      <tr className={trClass}>
                        <td colSpan={6} className="bg-surface-elevated px-[18px] py-4">
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
                );
              })}
              {items.length === 0 && (
                <tr>
                  <td colSpan={canWrite ? 6 : 4} className="px-4 py-6 text-center text-text-hint">
                    No users.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>

        {canWrite && (
          <p className="max-w-[620px] font-mono text-[12.5px] text-text-hint text-pretty">
            <span className="text-text-dim">keys</span> sets a user's agent credentials for them, so a workspace can be
            handed over ready to use. <span className="text-error">✕</span> destroys the container and its volume, that
            data is gone.
          </p>
        )}
      </PageBody>
    </>
  );
}

/// Small mono text action used in the workspaces table's actions column (open,
/// start, stop, keys, destroy). Not a `Button`: those carry border/height
/// chrome this compact, inline row of actions does not want.
function RowAction({
  onClick,
  disabled,
  tone = "faint",
  title,
  children,
}: {
  onClick: () => void;
  disabled?: boolean;
  tone?: "faint" | "active" | "danger";
  title?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      className={clsx(
        "rounded-sm font-mono text-[11.5px] transition-colors",
        "focus-visible:ring-2 focus-visible:ring-accent/40 focus-visible:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-40",
        tone === "danger" && "text-error hover:text-error",
        tone === "active" && "text-accent",
        tone === "faint" && "text-text-faint hover:text-text",
      )}
    >
      {children}
    </button>
  );
}

function MemberWorkspace({ me }: { me: Me }) {
  const [proxyOrigin, setProxyOrigin] = useState<string | null>(null);
  const [gitDone, setGitDone] = useState(false);
  const [agentDone, setAgentDone] = useState(false);
  const [firstRunOpen, setFirstRunOpen] = useState(false);

  useEffect(() => {
    api
      .myWorkspace()
      .then((w) => setProxyOrigin(w.proxy_origin))
      .catch(() => {});
  }, []);

  useEffect(() => {
    Promise.allSettled([api.getGitCredential(), api.getGitSshKey(), api.getAgentCredentials()]).then(
      ([gitRes, sshRes, agentRes]) => {
        const gitOk = gitRes.status === "fulfilled" && gitRes.value.token_set;
        const sshOk = sshRes.status === "fulfilled" && sshRes.value.key_set;
        setGitDone(gitOk || sshOk);
        setAgentDone(agentRes.status === "fulfilled" && agentRes.value.credentials.some((c) => c.value_set));
      },
    );
  }, []);

  const passwordDone = !me.must_change_password;
  const doneCount = [passwordDone, gitDone, agentDone].filter(Boolean).length;

  useEffect(() => {
    if (!me.onboarding_dismissed && !onboardingDismissedInSession.has(me.id)) setFirstRunOpen(true);
  }, [me.id, me.onboarding_dismissed]);

  function openWorkspace() {
    if (proxyOrigin) window.open(`${proxyOrigin}/?cityhall_ws_exit=1`, "_blank", "noopener");
  }

  function closeFirstRun() {
    onboardingDismissedInSession.add(me.id);
    setFirstRunOpen(false);
  }

  return (
    <>
      <PageHeader title="My workspace" meta="docker backend" />
      <PageBody className="max-w-[880px]">
        <div className="flex flex-col gap-[18px]">
          <div className="flex items-start justify-between gap-6">
            <div className="flex flex-col gap-2">
              <h2 className="m-0 text-[22px] font-semibold tracking-[-0.02em] text-text-bright">
                Your workspace is ready
              </h2>
              <p className="m-0 max-w-[460px] text-sm leading-relaxed text-text-dim text-pretty">
                A private aoe instance, yours alone, provisioned with the projects and coding agents your admin
                selected.
              </p>
            </div>
            <Button variant="primary" className="h-[38px] shrink-0" disabled={!proxyOrigin} onClick={openWorkspace}>
              Open workspace <ExternalLink size={14} />
            </Button>
          </div>

          <div className="grid grid-cols-3 gap-3">
            <InfoCard
              eyebrow="YOU OPEN IT"
              body="The container starts on the first request. There is no start button."
            />
            <InfoCard
              eyebrow="IDLE TIMEOUT"
              body="It stops. Files, sessions and agent logins are kept on your volume."
            />
            <InfoCard eyebrow="YOU COME BACK" body="It resumes where you left it. An open session is never cut off." />
          </div>

          <div className={tableCardClass}>
            <div className="flex items-center justify-between border-b border-border-soft px-[17px] py-[13px]">
              <SectionLabel>Before your first session · {doneCount} of 3</SectionLabel>
              <Button variant="text" onClick={() => setFirstRunOpen(true)}>
                Walk me through it
              </Button>
            </div>
            <ChecklistRow done={passwordDone} label="Password set" />
            <ChecklistRow
              done={gitDone}
              label={
                <>
                  Connect git, a token or an SSH key for <code className="font-mono text-xs">git@</code> remotes
                </>
              }
            />
            <ChecklistRow done={agentDone} label="Give your agent a credential" last />
          </div>
        </div>
      </PageBody>

      {firstRunOpen && <FirstRunModal proxyOrigin={proxyOrigin} onFinish={closeFirstRun} />}
    </>
  );
}

function InfoCard({ eyebrow, body }: { eyebrow: string; body: string }) {
  return (
    <Card>
      <SectionLabel>{eyebrow}</SectionLabel>
      <p className="m-0 mt-[7px] text-[13px] leading-relaxed text-text-dim">{body}</p>
    </Card>
  );
}

function ChecklistRow({ done, label, last }: { done: boolean; label: ReactNode; last?: boolean }) {
  const row = (
    <div
      className={clsx(
        "flex items-center gap-2.5 px-[17px] py-3",
        !last && "border-b border-border-soft",
        !done && "cursor-pointer transition-colors hover:bg-surface-hover",
      )}
    >
      <span className={clsx("font-mono text-xs", done ? "text-running" : "text-waiting")}>{done ? "✓" : "○"}</span>
      <span className={clsx("text-[13.5px]", done ? "text-text-dim" : "text-text")}>{label}</span>
      {!done && <span className="ml-auto font-mono text-[11.5px] text-link">Set up →</span>}
    </div>
  );
  return done ? row : <Link to="/account">{row}</Link>;
}
