import clsx from "clsx";
import type { Dashboard, MetricScope, WorkspaceUsage } from "../../lib/api";
import { formatBytes, formatPercent } from "../../lib/dashboardLayout";
import { Meter, SectionLabel, StatusText, tableHeadClass, thClass, trClass, tdClass } from "../ui";
import { WORKSPACE_STATUS_META } from "../../lib/workspaceStatus";

/// What the system figures actually measure. Shown next to them because
/// CityHall usually runs in a container beside the workspaces it reports on,
/// where these are not host figures and reading them as such is how an incident
/// gets misdiagnosed.
const SCOPE_LABELS: Record<MetricScope, string> = {
  host: "CityHall host",
  cityhall_container: "System visible to CityHall",
};

function Stat({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div>
      <SectionLabel>{label}</SectionLabel>
      <p className="mt-1 text-2xl font-light tracking-tight text-text-bright">{value}</p>
    </div>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="text-sm text-text-dim">{children}</p>;
}

export function SystemUsageWidget({ data }: { data: Dashboard }) {
  if (!data.system) return <Empty>No system figures in the latest sample.</Empty>;
  const { scope, cpu_percent, cpu_count, memory_used_bytes, memory_total_bytes, cgroup_memory, disk } = data.system;
  return (
    <div className="space-y-4">
      <p className="text-xs text-text-dim">
        {SCOPE_LABELS[scope]}
        {scope === "cityhall_container" && " (not necessarily the workspace host)"}
      </p>
      <div className="grid grid-cols-2 gap-4">
        <Stat label={`CPU (${cpu_count} cores)`} value={formatPercent(cpu_percent)} />
        <Stat
          label="Memory"
          value={
            <>
              {formatBytes(memory_used_bytes)}{" "}
              <span className="text-base font-normal text-text-dim">/ {formatBytes(memory_total_bytes)}</span>
            </>
          }
        />
      </div>
      <Meter percent={memory_total_bytes > 0 ? (memory_used_bytes / memory_total_bytes) * 100 : 0} danger />
      {cgroup_memory && (
        <div className="space-y-1.5">
          <SectionLabel>CityHall memory limit</SectionLabel>
          <p className="text-sm text-text">
            {formatBytes(cgroup_memory.used_bytes)} / {formatBytes(cgroup_memory.limit_bytes)}
          </p>
          <Meter
            percent={cgroup_memory.limit_bytes > 0 ? (cgroup_memory.used_bytes / cgroup_memory.limit_bytes) * 100 : 0}
            danger
          />
        </div>
      )}
      {disk ? (
        <div className="space-y-1.5">
          <SectionLabel>Disk {disk.mount_point}</SectionLabel>
          <p className="text-sm text-text">
            {formatBytes(disk.used_bytes)} / {formatBytes(disk.total_bytes)}
          </p>
          <Meter percent={disk.total_bytes > 0 ? (disk.used_bytes / disk.total_bytes) * 100 : 0} danger />
        </div>
      ) : (
        <p className="text-xs text-text-hint">
          Set <span className="font-mono text-text-dim">SYSTEM_METRICS_DISK_PATH</span> to report a filesystem.
        </p>
      )}
    </div>
  );
}

export function FleetStatusWidget({ data }: { data: Dashboard }) {
  const { summary } = data;
  return (
    <div className="grid grid-cols-3 gap-4">
      <Stat label="Running" value={<span className="text-running">{summary.running}</span>} />
      <Stat label="Stopped" value={String(summary.stopped)} />
      <Stat label="Not created" value={<span className="text-text-dim">{summary.not_created}</span>} />
      <Stat label="Users" value={String(summary.total_users)} />
      <Stat label="Provisioning" value={<span className="text-text-dim">{summary.provisioning}</span>} />
      <Stat label="Errors" value={<span className="text-text-dim">{summary.unknown}</span>} />
    </div>
  );
}

export function ContainersWidget({ data }: { data: Dashboard }) {
  const usage = new Map<number, WorkspaceUsage>(data.usage.map((u) => [u.user_id, u]));
  if (data.workspaces.length === 0) return <Empty>No users.</Empty>;
  return (
    <div className="space-y-2">
      {!data.usage_supported && (
        <p className="text-xs text-text-hint">Resource usage is not available on this workspace backend.</p>
      )}
      <table className="w-full text-sm">
        <thead>
          <tr className={tableHeadClass}>
            <th className={thClass}>User</th>
            <th className={thClass}>Status</th>
            <th className={thClass}>Version</th>
            {data.usage_supported && <th className={clsx(thClass, "text-right")}>CPU</th>}
            {data.usage_supported && <th className={clsx(thClass, "text-right")}>Memory</th>}
          </tr>
        </thead>
        <tbody>
          {data.workspaces.map((w) => {
            const u = usage.get(w.user_id);
            return (
              <tr key={w.user_id} className={trClass}>
                <td className={clsx(tdClass, "text-text-bright")}>{w.username}</td>
                <td className={tdClass}>
                  {w.provisioning ? (
                    <StatusText
                      tone={w.provisioning.failed ? "error" : "waiting"}
                      glyph={w.provisioning.failed ? "✕" : "◐"}
                      title={w.provisioning.message}
                    >
                      {w.provisioning.failed ? "provisioning failed" : "provisioning"}
                    </StatusText>
                  ) : (
                    <StatusText
                      tone={WORKSPACE_STATUS_META[w.status].tone}
                      glyph={WORKSPACE_STATUS_META[w.status].glyph}
                    >
                      {WORKSPACE_STATUS_META[w.status].label}
                    </StatusText>
                  )}
                </td>
                <td className={clsx(tdClass, "text-text-dim")}>
                  {w.effective_version ?? "-"}
                  {/* Only worth showing when they disagree, which means a version
                      change is waiting for a restart. */}
                  {w.running_version && w.running_version !== w.effective_version && (
                    <span className="ml-1.5 text-xs text-waiting" title="running until the next restart">
                      running {w.running_version}
                    </span>
                  )}
                </td>
                {data.usage_supported && (
                  <td className={clsx(tdClass, "text-right text-text-dim")}>
                    {u ? formatPercent(u.cpu_percent) : "-"}
                  </td>
                )}
                {data.usage_supported && (
                  <td className={clsx(tdClass, "text-right text-text-dim")}>{u ? formatBytes(u.memory_bytes) : "-"}</td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function VersionsWidget({ data }: { data: Dashboard }) {
  if (data.versions.length === 0) return <Empty>No workspace version configured.</Empty>;
  const total = data.versions.reduce((sum, v) => sum + v.count, 0);
  return (
    <div className="space-y-2.5">
      {data.versions.map((v) => (
        <div key={v.version} className="space-y-1">
          <div className="flex justify-between text-sm">
            <span className="text-text">{v.version}</span>
            <span className="text-text-dim">{v.count}</span>
          </div>
          <Meter percent={total > 0 ? (v.count / total) * 100 : 0} />
        </div>
      ))}
    </div>
  );
}

export function ProvisioningWidget({ data }: { data: Dashboard }) {
  const jobs = data.workspaces.flatMap((w) => (w.provisioning ? [{ w, info: w.provisioning }] : []));
  if (jobs.length === 0) return <Empty>Nothing provisioning.</Empty>;
  return (
    <div className="space-y-1.5 text-sm">
      {jobs.map(({ w, info }) => (
        <p key={w.user_id} className={info.failed ? "text-error" : "text-waiting"}>
          <span className="font-medium text-text-bright">{w.username}</span>: {info.message}
        </p>
      ))}
      <p className="text-xs text-text-hint">
        A first image pull or local build takes a few minutes and continues if this page is closed.
      </p>
    </div>
  );
}

/// Maps a catalog widget id to what renders inside its card.
export const WIDGET_BODIES: Record<string, (props: { data: Dashboard }) => React.ReactNode> = {
  "system-usage": SystemUsageWidget,
  "fleet-status": FleetStatusWidget,
  containers: ContainersWidget,
  versions: VersionsWidget,
  provisioning: ProvisioningWidget,
};
