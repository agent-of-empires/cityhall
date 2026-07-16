import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type WorkspaceSettings } from "../lib/api";
import { isOlderVersion } from "../lib/versions";
import { Button, ErrorText, Field, Input, Select } from "./ui";

export function WorkspaceSettingsSection() {
  const [loadError, setLoadError] = useState<string | null>(null);

  const [imageTemplate, setImageTemplate] = useState("");
  const [defaultVersion, setDefaultVersion] = useState("");
  const [idleStopMinutes, setIdleStopMinutes] = useState(30);
  const [versions, setVersions] = useState<string[]>([]);
  const [latest, setLatest] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const apply = useCallback((s: WorkspaceSettings) => {
    setImageTemplate(s.image_template);
    setDefaultVersion(s.default_version ?? "");
    setIdleStopMinutes(s.idle_stop_minutes);
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
          <Field label="Default version">
            {versions.length > 0 ? (
              <Select value={defaultVersion} onChange={(e) => setDefaultVersion(e.target.value)}>
                <option value="">none</option>
                {/* A previously saved version can predate the discovered list. */}
                {defaultVersion && !versions.includes(defaultVersion) && (
                  <option value={defaultVersion}>{defaultVersion}</option>
                )}
                {versions.map((v) => (
                  <option key={v} value={v}>
                    {v}
                    {v === latest ? " (latest)" : ""}
                  </option>
                ))}
              </Select>
            ) : (
              <Input value={defaultVersion} onChange={(e) => setDefaultVersion(e.target.value)} placeholder="v0.1.0" />
            )}
          </Field>
          <Field label="Idle stop (minutes)">
            <Input
              type="number"
              value={idleStopMinutes}
              onChange={(e) => setIdleStopMinutes(Number(e.target.value))}
              min={1}
            />
          </Field>
        </div>

        <p className="text-sm text-text-secondary">
          The image for a user is the template with <code className="text-text-primary">{"{version}"}</code> replaced by
          their pinned version (or the default). Idle workspaces are stopped automatically; their data volume is kept.
        </p>

        {latest && defaultVersion && isOlderVersion(defaultVersion, latest) && (
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
