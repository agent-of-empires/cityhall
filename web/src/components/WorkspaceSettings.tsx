import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type TelemetryPolicy, type WorkspaceSettings } from "../lib/api";
import { isOlderVersion } from "../lib/versions";
import { Button, ErrorText, Field, Input, Select } from "./ui";
import { VersionField } from "./VersionField";

const TELEMETRY_LABELS: Record<TelemetryPolicy, string> = {
  user_choice: "Let each user choose",
  force_on: "On for everyone",
  force_off: "Off for everyone",
};

/// What to say when `WORKSPACE_TELEMETRY_POLICY` pins the policy, or null when
/// nothing is pinned. The stored value is still saveable while pinned, so the
/// note has to say that the choice is kept rather than applied.
export function telemetryOverrideNote(override: TelemetryPolicy | null): string | null {
  if (!override) return null;
  return `Pinned to "${TELEMETRY_LABELS[override]}" by WORKSPACE_TELEMETRY_POLICY in this deployment's environment. A saved choice is stored for when that variable is removed, and does not apply while it is set.`;
}

export function WorkspaceSettingsSection() {
  const [loadError, setLoadError] = useState<string | null>(null);

  const [imageTemplate, setImageTemplate] = useState("");
  const [defaultVersion, setDefaultVersion] = useState("");
  const [idleStopMinutes, setIdleStopMinutes] = useState(30);
  const [telemetryPolicy, setTelemetryPolicy] = useState<TelemetryPolicy>("user_choice");
  const [telemetryOverride, setTelemetryOverride] = useState<TelemetryPolicy | null>(null);
  const [restartRunning, setRestartRunning] = useState(false);
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
  }, []);

  const load = useCallback(async () => {
    try {
      apply(await api.getWorkspaceSettings());
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
        }),
      );
      setSaved(true);
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not save workspace settings");
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      <h2 className="mt-4 font-mono text-xs uppercase tracking-wider text-text-muted">Workspaces</h2>

      {loadError && <ErrorText>{loadError}</ErrorText>}

      <form onSubmit={save} className="space-y-4 rounded-lg border border-surface-700 p-5">
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field label="Image template">
            <Input
              value={imageTemplate}
              onChange={(e) => setImageTemplate(e.target.value)}
              placeholder="cityhall/aoe:{version}"
            />
          </Field>
          {/* Deliberately not a Field: that wraps its children in a label with
              no `for`, which binds to the first control inside it, and here that
              would be VersionField's checkbox rather than the version control. */}
          <div className="block space-y-1.5">
            <span className="font-mono text-xs uppercase tracking-wider text-text-muted">Default version</span>
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
          <Field label="aoe telemetry">
            <Select value={telemetryPolicy} onChange={(e) => setTelemetryPolicy(e.target.value as TelemetryPolicy)}>
              {(Object.keys(TELEMETRY_LABELS) as TelemetryPolicy[]).map((policy) => (
                <option key={policy} value={policy}>
                  {TELEMETRY_LABELS[policy]}
                </option>
              ))}
            </Select>
          </Field>
        </div>

        {telemetryOverride && <p className="text-sm text-status-waiting">{telemetryOverrideNote(telemetryOverride)}</p>}

        {telemetryPolicy === "force_on" && (
          <p className="text-sm text-status-waiting">
            Turning telemetry on for everyone suppresses aoe's consent prompt and records the choice as answered in each
            user's workspace, so disclosing the collection to your users is your deployment's responsibility. Reverting
            to "Let each user choose" stops enforcing it, but leaves those users opted in until they change it
            themselves.
          </p>
        )}

        <p className="text-sm text-text-secondary">
          A telemetry change reaches a workspace the next time it starts.{" "}
          <label className="inline-flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={restartRunning}
              onChange={(e) => setRestartRunning(e.target.checked)}
              className="h-4 w-4 accent-brand-500"
            />
            restart running workspaces on save
          </label>{" "}
          to apply it now, ending whatever their users are running.
        </p>

        <p className="text-sm text-text-secondary">
          The image for a user is the template with <code className="text-text-primary">{"{version}"}</code> replaced by
          their pinned version (or the default). Idle workspaces are stopped automatically; their data volume is kept.
        </p>

        {/* Only releases are comparable: a custom tag's digits are not a
            version, so "dev-0" would otherwise read as behind the latest. */}
        {latest && versions.includes(defaultVersion) && isOlderVersion(defaultVersion, latest) && (
          <p className="text-sm text-status-waiting">
            The default version {defaultVersion} is behind the latest release {latest}.{" "}
            <button type="button" className="underline" onClick={() => setDefaultVersion(latest)}>
              Use {latest}
            </button>
          </p>
        )}

        {saveError && <ErrorText>{saveError}</ErrorText>}
        {saved && <p className="text-sm text-status-running">Settings saved.</p>}

        <div className="flex justify-end">
          <Button type="submit" variant="primary" disabled={saving}>
            {saving ? "Saving..." : "Save settings"}
          </Button>
        </div>
      </form>
    </>
  );
}
