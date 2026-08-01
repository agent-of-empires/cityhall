import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type WorkspaceConfig } from "../lib/api";
import { Button, ErrorText } from "./ui";

/// The aoe config bundle every workspace is provisioned with (#9).
///
/// A text editor rather than a form: aoe defines the settings schema, so aoe
/// owns the format, and rendering real widgets would mean reimplementing aoe's
/// generic field renderer here. The document is already human-editable, and the
/// server shape-checks it on save.
/// Seed for an admin who has no aoe install to export from.
///
/// Everything past `schema_version` is commented out on purpose: this saves as a
/// valid document that provisions nothing, so someone can start here and add one
/// project without a half-filled example cloning `org/my-repo` into every
/// workspace on the next start.
const BLANK_BUNDLE = `schema_version = 1

# Settings every workspace starts with. These are aoe's own setting names, so the
# easiest way to get real ones is Settings -> CityHall in an aoe install, which
# generates this file for you. Uncomment to use.
# [settings.acp]
# default_agent = "claude-code"

# Repos cloned into every workspace on its next start. Addressed by git remote,
# not by path, because an admin's local checkout means nothing in a container.
# [[projects]]
# name = "my-repo"
# remote = "https://github.com/org/my-repo.git"
# default_base_branch = "main"
`;

export function WorkspaceConfigSection() {
  const [config, setConfig] = useState<WorkspaceConfig | null>(null);
  const [bundle, setBundle] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const apply = useCallback((c: WorkspaceConfig) => {
    setConfig(c);
    setBundle(c.bundle);
  }, []);

  const load = useCallback(async () => {
    try {
      apply(await api.getWorkspaceConfig());
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof ApiError ? e.message : "could not load the workspace config");
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
      apply(await api.updateWorkspaceConfig(bundle));
      setSaved(true);
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not save the workspace config");
    } finally {
      setSaving(false);
    }
  }

  async function upload(file: File) {
    setSaved(false);
    setSaveError(null);
    try {
      setBundle(await file.text());
    } catch {
      // The picker hands back a handle, not bytes, so a file that moved or
      // became unreadable between selection and read rejects here. The caller
      // fires this without awaiting, so an unhandled rejection is the
      // alternative to reporting it.
      setSaveError("could not read the selected file");
    }
  }

  const summary = config?.summary;

  return (
    <>
      <h2 className="mt-4 font-mono text-xs uppercase tracking-wider text-text-muted">Workspace config</h2>

      {loadError && <ErrorText>{loadError}</ErrorText>}

      <form onSubmit={save} className="space-y-4 rounded-lg border border-surface-700 p-5">
        <p className="text-sm text-text-secondary">
          The aoe settings and projects every workspace is provisioned with. Generate one from a configured aoe install
          with <code className="text-text-primary">aoe cityhall export</code>, or from its dashboard under Settings
          &rarr; CityHall, then paste it here. You do not need an aoe install to start: write one here instead. Projects
          are cloned into each workspace on its next start, so a user has something to launch a session against.
        </p>

        <div className="flex flex-wrap items-center gap-3">
          {/* sr-only, not hidden: display:none takes the input out of the tab
              order, and the label is not focusable, so the control would be
              pointer-only. */}
          <label className="cursor-pointer rounded-sm text-sm text-text-secondary underline hover:text-text-primary focus-within:outline-none focus-within:ring-2 focus-within:ring-brand-500">
            Upload cityhall.toml
            <input
              type="file"
              accept=".toml,text/plain"
              className="sr-only"
              onChange={(e) => {
                const file = e.target.files?.[0];
                // Clear the input so re-picking the same file still fires.
                e.target.value = "";
                if (file) void upload(file);
              }}
            />
          </label>
          {/* Only offered while there is nothing to lose: seeding over a real
              document would discard an admin's work to a stray click. */}
          {!bundle.trim() && (
            <button
              type="button"
              className="cursor-pointer rounded-sm text-sm text-text-secondary underline hover:text-text-primary"
              onClick={() => {
                setBundle(BLANK_BUNDLE);
                setSaved(false);
                setSaveError(null);
              }}
            >
              Start from scratch
            </button>
          )}
          {config?.updated_at && (
            <span className="text-sm text-text-muted">Last saved {new Date(config.updated_at).toLocaleString()}</span>
          )}
        </div>

        <textarea
          aria-label="Workspace config bundle"
          value={bundle}
          onChange={(e) => {
            setBundle(e.target.value);
            setSaved(false);
          }}
          spellCheck={false}
          rows={16}
          placeholder={
            'schema_version = 1\n\n[[projects]]\nname = "my-repo"\nremote = "https://github.com/org/my-repo.git"'
          }
          className="w-full rounded-md border border-surface-700 bg-surface-950 px-3 py-2 font-mono text-xs text-text-primary placeholder:text-text-muted focus:border-brand-500 focus:outline-none focus-visible:ring-2 focus-visible:ring-brand-500"
        />

        {summary && (summary.settings_count > 0 || summary.projects.length > 0) && (
          <p className="text-sm text-text-secondary">
            Saved bundle: {summary.settings_count} overridden {summary.settings_count === 1 ? "setting" : "settings"}
            {summary.projects.length > 0 && <>, projects: {summary.projects.join(", ")}</>}.
          </p>
        )}

        <p className="text-sm text-text-secondary">
          Changes reach a workspace the next time it starts; restart one from the Workspaces page to apply them now.
          Each user's git identity and credential are added when the bundle is served, so this document never holds a
          secret.
        </p>

        {saveError && <ErrorText>{saveError}</ErrorText>}
        {saved && <p className="text-sm text-status-running">Workspace config saved.</p>}

        <div className="flex justify-end">
          <Button type="submit" variant="primary" disabled={saving}>
            {saving ? "Saving..." : "Save config"}
          </Button>
        </div>
      </form>
    </>
  );
}
