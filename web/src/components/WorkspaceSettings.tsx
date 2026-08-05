import { useCallback, useEffect, useState, type ReactNode } from "react";
import { api, ApiError, type AvailableAgent, type TelemetryPolicy, type WorkspaceSettings } from "../lib/api";
import { isOlderVersion } from "../lib/versions";
import { Button, Card, Checkbox, ErrorText, Field, Input, Select } from "./ui";
import { VersionField } from "./VersionField";

const TELEMETRY_LABELS: Record<TelemetryPolicy, string> = {
  user_choice: "Let each user choose",
  force_on: "On for everyone",
  force_off: "Off for everyone",
};

/// One "label + help on the left, control on the right" settings row, matching
/// the mockup's c-setting pattern. Kept local rather than shared from
/// SettingsPage.tsx, which imports this section and would make a cycle.
function SettingRow({ label, help, children }: { label: string; help?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-6">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-accent">{label}</div>
        {help && <p className="mt-1 max-w-[420px] text-[12.5px] leading-relaxed text-text-dim">{help}</p>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

/// What to say when `WORKSPACE_TELEMETRY_POLICY` pins the policy, or null when
/// nothing is pinned. The stored value is still saveable while pinned, so the
/// note has to say that the choice is kept rather than applied.
export function telemetryOverrideNote(override: TelemetryPolicy | null): string | null {
  if (!override) return null;
  return `Pinned to "${TELEMETRY_LABELS[override]}" by WORKSPACE_TELEMETRY_POLICY in this deployment's environment. A saved choice is stored for when that variable is removed, and does not apply while it is set.`;
}

export function isKnownTelemetryPolicy(value: string): value is TelemetryPolicy {
  return value in TELEMETRY_LABELS;
}

/// The policy workspaces would run under: the environment override when there is
/// one, otherwise whatever is currently selected.
///
/// Selected, not saved, so the force-on disclosure appears while the admin is
/// choosing it rather than only after the fact. The server computes the same
/// thing from the stored value; reading its answer here instead would be a
/// save behind whatever the form shows.
///
/// A selection this build does not know is a policy written by a newer CityHall,
/// which the server also reads as `user_choice`, so neither forced-state notice
/// claims something that is not being enforced here.
export function effectiveTelemetryPolicy(selected: string, override: TelemetryPolicy | null): TelemetryPolicy {
  if (override) return override;
  return isKnownTelemetryPolicy(selected) ? selected : "user_choice";
}

/// Tick or untick one agent, keeping the selection in the catalog's order.
///
/// Order matters because the server stores a canonical set: sending the same
/// agents in a different order would otherwise read as a change and recreate
/// every workspace for nothing.
export function toggleAgent(selected: string[], catalog: AvailableAgent[], name: string, on: boolean): string[] {
  const next = new Set(selected);
  if (on) next.add(name);
  else next.delete(name);
  return catalog.filter((a) => next.has(a.name)).map((a) => a.name);
}

/// Workspace defaults (#61) and the image template (moved to the Advanced tab)
/// are two pages sharing one settings object and one save endpoint, so each
/// mounts its own copy of the full state and sends the whole thing back on
/// save; `section` only picks which fields it renders.
export function WorkspaceSettingsSection({ section }: { section: "defaults" | "advanced" }) {
  const [loadError, setLoadError] = useState<string | null>(null);
  // Saving sends every field, so until the first read lands the form holds
  // defaults (no image template, no agents, user_choice) that would overwrite a
  // real deployment's settings on an eager click.
  const [loaded, setLoaded] = useState(false);

  const [imageTemplate, setImageTemplate] = useState("");
  const [defaultVersion, setDefaultVersion] = useState("");
  const [idleStopMinutes, setIdleStopMinutes] = useState(30);
  // A plain string, not a TelemetryPolicy: a policy stored by a newer CityHall
  // has to round-trip through this form untouched.
  const [telemetryPolicy, setTelemetryPolicy] = useState<string>("user_choice");
  const [telemetryOverride, setTelemetryOverride] = useState<TelemetryPolicy | null>(null);
  const [restartRunning, setRestartRunning] = useState(false);
  const [agents, setAgents] = useState<string[]>([]);
  const [availableAgents, setAvailableAgents] = useState<AvailableAgent[]>([]);
  const [versions, setVersions] = useState<string[]>([]);
  const [latest, setLatest] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const apply = useCallback((s: WorkspaceSettings) => {
    setImageTemplate(s.image_template);
    setDefaultVersion(s.default_version ?? "");
    setIdleStopMinutes(s.idle_stop_minutes);
    setTelemetryPolicy(s.telemetry_policy);
    setTelemetryOverride(s.telemetry_policy_override);
    // Defaulted rather than trusted: a response missing either list would
    // otherwise throw during render and take the whole settings page down,
    // including the sections that have nothing to do with workspaces.
    setAgents(s.agents ?? []);
    setAvailableAgents(s.available_agents ?? []);
  }, []);

  const load = useCallback(async () => {
    try {
      apply(await api.getWorkspaceSettings());
      setLoaded(true);
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof ApiError ? e.message : "could not load workspace settings");
    }
  }, [apply]);

  useEffect(() => {
    void load();
    api
      .listWorkspaceVersions()
      .then((v) => {
        setVersions(v.versions);
        setLatest(v.latest);
      })
      .catch(() => {});
  }, [load]);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      apply(
        await api.updateWorkspaceSettings({
          image_template: imageTemplate,
          default_version: defaultVersion.trim() || null,
          idle_stop_minutes: idleStopMinutes,
          telemetry_policy: telemetryPolicy,
          restart_running: restartRunning,
          agents,
        }),
      );
      setSaved(true);
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not save workspace settings");
    } finally {
      setSaving(false);
    }
  }

  if (section === "advanced") {
    return (
      <div className="flex flex-col gap-4">
        {loadError && <ErrorText>{loadError}</ErrorText>}

        <Card>
          <form onSubmit={save} className="flex flex-col gap-4">
            <Field
              label="Image template"
              help="Point this at a registry your cluster can pull from. On kubernetes a missing image cannot be built locally. The image for a user is the template with {version} replaced by their pinned version (or the default)."
            >
              <Input
                value={imageTemplate}
                onChange={(e) => setImageTemplate(e.target.value)}
                placeholder="cityhall/aoe:{version}"
              />
            </Field>

            {/* The image only changes for a workspace that is recreated, so this
                tab needs its own copy of the restart opt-in. */}
            <Checkbox
              checked={restartRunning}
              onChange={setRestartRunning}
              label="Restart running workspaces on save, to apply it now, ending whatever their users are running."
            />

            {saveError && <ErrorText>{saveError}</ErrorText>}
            {saved && <p className="text-sm text-running">Settings saved.</p>}

            <div className="flex justify-end">
              <Button type="submit" variant="primary" disabled={saving || !loaded}>
                {saving ? "Saving..." : "Save settings"}
              </Button>
            </div>
          </form>
        </Card>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {loadError && <ErrorText>{loadError}</ErrorText>}

      <Card>
        <form onSubmit={save} className="flex flex-col gap-4">
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            {/* Deliberately not a Field: that wraps its children in a label with
                no `for`, which binds to the first control inside it, and here that
                would be VersionField's checkbox rather than the version control. */}
            <div className="block space-y-1.5">
              <span className="block font-mono text-[10.5px] tracking-[0.12em] text-text-hint uppercase">
                Default version
              </span>
              <VersionField
                value={defaultVersion}
                onChange={setDefaultVersion}
                versions={versions}
                latest={latest ?? undefined}
                noneLabel="none"
              />
            </div>
            <Field label="Idle stop (minutes)">
              <Input
                type="number"
                value={idleStopMinutes}
                onChange={(e) => setIdleStopMinutes(Number(e.target.value))}
                min={1}
              />
            </Field>
          </div>

          {latest && versions.includes(defaultVersion) && isOlderVersion(defaultVersion, latest) && (
            <p className="text-[12.5px] text-waiting">
              The default version {defaultVersion} is behind the latest release {latest}.{" "}
              <button type="button" className="underline" onClick={() => setDefaultVersion(latest)}>
                Use {latest}
              </button>
            </p>
          )}

          <p className="text-[12.5px] leading-relaxed text-text-dim">
            Idle workspaces are stopped automatically; their data volume is kept.
          </p>

          <div className="flex flex-col gap-2 border-t border-border-soft pt-4">
            <span className="font-mono text-[10.5px] tracking-[0.12em] text-text-hint uppercase">
              Coding agents preinstalled
            </span>
            <div className="flex flex-wrap gap-x-6 gap-y-2">
              {availableAgents.map((agent) => (
                <Checkbox
                  key={agent.name}
                  checked={agents.includes(agent.name)}
                  onChange={(on) => setAgents(toggleAgent(agents, availableAgents, agent.name, on))}
                  label={agent.label}
                />
              ))}
            </div>
            <p className="text-[12.5px] leading-relaxed text-text-hint">
              Installed the first time a workspace starts, so a user gets one that is ready to use. Leave all of them
              unticked and users install their own instead. A change reaches an existing workspace the next time it is
              created, which is a restart, an idle stop, or a first launch. This is a default and not a restriction: a
              user can still install and run any agent they like.
            </p>
          </div>

          {saveError && <ErrorText>{saveError}</ErrorText>}
          {saved && <p className="text-sm text-running">Settings saved.</p>}

          <div className="flex justify-end">
            <Button type="submit" variant="primary" disabled={saving || !loaded}>
              {saving ? "Saving..." : "Save settings"}
            </Button>
          </div>
        </form>
      </Card>

      <Card>
        <SettingRow
          label="aoe telemetry"
          help="aoe asks each user to opt in inside their own workspace. In a deployment that is the wrong person to ask."
        >
          <div className="w-56">
            <Select value={telemetryPolicy} onChange={(e) => setTelemetryPolicy(e.target.value)}>
              {/* Keeps an unrecognized stored policy selectable, so leaving the
                  control alone saves it back verbatim instead of the browser
                  silently falling back to the first option. */}
              {!isKnownTelemetryPolicy(telemetryPolicy) && (
                <option value={telemetryPolicy}>Set by a newer CityHall ({telemetryPolicy})</option>
              )}
              {(Object.keys(TELEMETRY_LABELS) as TelemetryPolicy[]).map((policy) => (
                <option key={policy} value={policy}>
                  {TELEMETRY_LABELS[policy]}
                </option>
              ))}
            </Select>
          </div>
        </SettingRow>

        {telemetryOverride && (
          <p className="mt-3 text-[12.5px] text-waiting">{telemetryOverrideNote(telemetryOverride)}</p>
        )}

        {effectiveTelemetryPolicy(telemetryPolicy, telemetryOverride) === "force_on" && (
          <p className="mt-3 text-[12.5px] leading-relaxed text-waiting">
            Turning telemetry on for everyone suppresses aoe's consent prompt and records the choice as answered in each
            user's workspace, so disclosing the collection to your users is your deployment's responsibility. Reverting
            to "Let each user choose" stops enforcing it, but leaves those users opted in until they change it
            themselves.
          </p>
        )}

        <p className="mt-3 text-[12.5px] leading-relaxed text-text-dim">
          A telemetry change reaches a workspace the next time it starts.
        </p>
        <div className="mt-2">
          <Checkbox
            checked={restartRunning}
            onChange={setRestartRunning}
            label="Restart running workspaces on save, to apply it now, ending whatever their users are running."
          />
        </div>
      </Card>
    </div>
  );
}
