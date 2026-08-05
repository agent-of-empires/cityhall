import clsx from "clsx";
import type { Dashboard, DashboardWorkspace, MetricScope, WorkspaceUsage } from "../../lib/api";
import { formatBytes, formatPercent } from "../../lib/dashboardLayout";

const STATUS_STYLES: Record<DashboardWorkspace["status"], string> = {
  running: "text-status-running",
  stopped: "text-status-waiting",
  not_created: "text-text-muted",
  unknown: "text-status-error",
};

const STATUS_LABELS: Record<DashboardWorkspace["status"], string> = {
  running: "running",
  stopped: "stopped",
  not_created: "not created",
  unknown: "unknown",
};

/// What the system figures actually measure. Shown next to them because
/// CityHall usually runs in a container beside the workspaces it reports on,
/// where these are not host figures and reading them as such is how an incident
/// gets misdiagnosed.
const SCOPE_LABELS: Record<MetricScope, string> = {
  host: "CityHall host",
  cityhall_container: "System visible to CityHall",
};

/// `danger` is opt-in because "nearly full" only means trouble for a capacity
/// bar. A version holding the whole fleet is the normal end state of a rollout,
/// and colouring that red reads as an alert about nothing.
function Meter({ used, total, danger = false }: { used: number; total: number; danger?: boolean }) {
  const percent = total > 0 ? Math.min(100, (used / total) * 100) : 0;
  return (
    <div className="h-1.5 overflow-hidden rounded-full bg-surface-800">
      <div
        className={clsx("h-full rounded-full", danger && percent > 90 ? "bg-status-error" : "bg-brand-500")}
        style={{ width: `${percent}%` }}
      />
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="font-mono text-xs uppercase tracking-wider text-text-muted">{label}</p>
      <p className="text-lg text-text-primary">{value}</p>
    </div>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="text-sm text-text-muted">{children}</p>;
}

export function SystemUsageWidget({ data }: { data: Dashboard }) {
  if (!data.system) return <Empty>No system figures in the latest sample.</Empty>;
  const { scope, cpu_percent, cpu_count, memory_used_bytes, memory_total_bytes, cgroup_memory, disk } = data.system;
  return (
    <div className="space-y-4">
      <p className="text-xs text-text-muted">
        {SCOPE_LABELS[scope]}
        {scope === "cityhall_container" && " (not necessarily the workspace host)"}
      </p>
      <div className="grid grid-cols-2 gap-4">
        <Stat label={`CPU (${cpu_count} cores)`} value={formatPercent(cpu_percent)} />
        <Stat label="Memory" value={`${formatBytes(memory_used_bytes)} / ${formatBytes(memory_total_bytes)}`} />
      </div>
      <Meter used={memory_used_bytes} total={memory_total_bytes} danger />
      {cgroup_memory && (
        <div className="space-y-1.5">
          <p className="font-mono text-xs uppercase tracking-wider text-text-muted">CityHall memory limit</p>
          <p className="text-sm text-text-secondary">
            {formatBytes(cgroup_memory.used_bytes)} / {formatBytes(cgroup_memory.limit_bytes)}
          </p>
          <Meter used={cgroup_memory.used_bytes} total={cgroup_memory.limit_bytes} danger />
        </div>
      )}
      {disk ? (
        <div className="space-y-1.5">
          <p className="font-mono text-xs uppercase tracking-wider text-text-muted">Disk {disk.mount_point}</p>
          <p className="text-sm text-text-secondary">
            {formatBytes(disk.used_bytes)} / {formatBytes(disk.total_bytes)}
          </p>
          <Meter used={disk.used_bytes} total={disk.total_bytes} danger />
        </div>
      ) : (
        <p className="text-xs text-text-muted">
          Set <code className="text-text-secondary">SYSTEM_METRICS_DISK_PATH</code> to report a filesystem.
        </p>
      )}
    </div>
  );
}

export function FleetStatusWidget({ data }: { data: Dashboard }) {
  const { summary } = data;
  return (
    <div className="grid grid-cols-2 gap-4">
      <Stat label="Running" value={String(summary.running)} />
      <Stat label="Stopped" value={String(summary.stopped)} />
      <Stat label="Not created" value={String(summary.not_created)} />
      <Stat label="Unknown" value={String(summary.unknown)} />
      <Stat label="Users" value={String(summary.total_users)} />
      <Stat label="Provisioning" value={String(summary.provisioning)} />
    </div>
  );
}

export function ContainersWidget({ data }: { data: Dashboard }) {
  const usage = new Map<number, WorkspaceUsage>(data.usage.map((u) => [u.user_id, u]));
  if (data.workspaces.length === 0) return <Empty>No users.</Empty>;
  return (
    <div className="space-y-2">
      {!data.usage_supported && (
        <p className="text-xs text-text-muted">Resource usage is not available on this workspace backend.</p>
      )}
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-surface-700 text-left font-mono text-xs uppercase tracking-wider text-text-muted">
            <th className="py-2 pr-4 font-medium">User</th>
            <th className="py-2 pr-4 font-medium">Status</th>
            <th className="py-2 pr-4 font-medium">Version</th>
            {data.usage_supported && <th className="py-2 pr-4 text-right font-medium">CPU</th>}
            {data.usage_supported && <th className="py-2 text-right font-medium">Memory</th>}
          </tr>
        </thead>
        <tbody>
          {data.workspaces.map((w) => {
            const u = usage.get(w.user_id);
            return (
              <tr key={w.user_id} className="border-b border-surface-800 last:border-0">
                <td className="py-2 pr-4 text-text-primary">{w.username}</td>
                <td className="py-2 pr-4">
                  {w.provisioning ? (
                    <span
                      className={w.provisioning.failed ? "text-status-error" : "text-status-waiting"}
                      title={w.provisioning.message}
                    >
                      {w.provisioning.failed ? "provisioning failed" : "provisioning"}
                    </span>
                  ) : (
                    <span className={STATUS_STYLES[w.status]}>{STATUS_LABELS[w.status]}</span>
                  )}
                </td>
                <td className="py-2 pr-4 text-text-secondary">
                  {w.effective_version ?? "-"}
                  {/* Only worth showing when they disagree, which means a version
                      change is waiting for a restart. */}
                  {w.running_version && w.running_version !== w.effective_version && (
                    <span className="ml-1.5 text-xs text-status-waiting" title="running until the next restart">
                      running {w.running_version}
                    </span>
                  )}
                </td>
                {data.usage_supported && (
                  <td className="py-2 pr-4 text-right text-text-secondary">{u ? formatPercent(u.cpu_percent) : "-"}</td>
                )}
                {data.usage_supported && (
                  <td className="py-2 text-right text-text-secondary">{u ? formatBytes(u.memory_bytes) : "-"}</td>
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
            <span className="text-text-primary">{v.version}</span>
            <span className="text-text-muted">{v.count}</span>
          </div>
          <Meter used={v.count} total={total} />
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
        <p key={w.user_id} className={info.failed ? "text-status-error" : "text-status-waiting"}>
          <span className="font-medium text-text-primary">{w.username}</span>: {info.message}
        </p>
      ))}
      <p className="text-xs text-text-muted">
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
