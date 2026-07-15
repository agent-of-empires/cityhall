import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type WorkspaceSettings } from "../lib/api";
import { Button, ErrorText, Field, Input } from "./ui";

export function WorkspaceSettingsSection() {
  const [loadError, setLoadError] = useState<string | null>(null);

  const [enabled, setEnabled] = useState(false);
  const [imageTemplate, setImageTemplate] = useState("");
  const [defaultVersion, setDefaultVersion] = useState("");
  const [idleStopMinutes, setIdleStopMinutes] = useState(30);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const apply = useCallback((s: WorkspaceSettings) => {
    setEnabled(s.enabled);
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
  }, [load]);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      apply(
        await api.updateWorkspaceSettings({
          enabled,
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
        <label className="flex items-center gap-2 text-sm text-text-primary">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
            className="h-4 w-4 accent-brand-500"
          />
          Enable per-user aoe workspaces
        </label>

        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field label="Image template">
            <Input
              value={imageTemplate}
              onChange={(e) => setImageTemplate(e.target.value)}
              placeholder="cityhall/aoe:{version}"
            />
          </Field>
          <Field label="Default version">
            <Input value={defaultVersion} onChange={(e) => setDefaultVersion(e.target.value)} placeholder="v0.1.0" />
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
