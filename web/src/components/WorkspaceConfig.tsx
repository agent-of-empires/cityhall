import { useCallback, useEffect, useMemo, useState } from "react";
import { api, ApiError, type WorkspaceConfig } from "../lib/api";
import {
  FIELDS,
  parseBundle,
  readProjects,
  readSetting,
  stringifyBundle,
  writeProjects,
  writeSetting,
  type Doc,
  type FormField,
  type ProjectRow,
} from "../lib/bundleForm";
import { Button, ErrorText, Input, Select } from "./ui";

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

/// The aoe config bundle every workspace is provisioned with (#9).
///
/// Two views over one document (#45). The TOML text is canonical and the form
/// is a structured view of it, so switching either way carries unsaved edits
/// and neither view can drift from the other. The form covers the settings that
/// matter without an aoe install in front of you; the text view stays because
/// it is how an export gets in and how a field CityHall does not know about gets
/// set. See `lib/bundleForm.ts`.
export function WorkspaceConfigSection() {
  const [config, setConfig] = useState<WorkspaceConfig | null>(null);
  const [bundle, setBundle] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [view, setView] = useState<"form" | "toml">("form");

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

  // Editing while a request is in flight would lose the edit: load and save both
  // reseed the editor from the response, so a reply that lands after a keystroke
  // overwrites it and then reports the stale document as saved.
  const busy = saving || config === null;

  async function upload(file: File) {
    setSaved(false);
    setSaveError(null);
    try {
      edit(await file.text());
    } catch {
      // The picker hands back a handle, not bytes, so a file that moved or
      // became unreadable between selection and read rejects here. The caller
      // fires this without awaiting, so an unhandled rejection is the
      // alternative to reporting it.
      setSaveError("could not read the selected file");
    }
  }

  function edit(next: string) {
    setBundle(next);
    setSaved(false);
  }

  const parsed = useMemo(() => parseBundle(bundle), [bundle]);
  const doc = "doc" in parsed ? parsed.doc : null;
  const parseError = "error" in parsed ? parsed.error : null;
  // Only warned about when there is something to lose: the form re-emits the
  // document from its values, so a comment does not survive an edit here.
  const hasComments = doc !== null && /^\s*#/m.test(bundle);

  function editDoc(next: Doc) {
    edit(stringifyBundle(next));
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
          &rarr; CityHall, then paste it here. You do not need an aoe install to start: fill in the form instead.
          Projects are cloned into each workspace on its next start, so a user has something to launch a session
          against.
        </p>

        <div className="flex flex-wrap items-center gap-3">
          <div className="flex gap-1 rounded-md border border-surface-700 p-1">
            {(["form", "toml"] as const).map((v) => (
              <Button
                key={v}
                type="button"
                variant={view === v ? "primary" : "ghost"}
                aria-pressed={view === v}
                onClick={() => setView(v)}
              >
                {v === "form" ? "Form" : "TOML"}
              </Button>
            ))}
          </div>
          {/* sr-only, not hidden: display:none takes the input out of the tab
              order, and the label is not focusable, so the control would be
              pointer-only. */}
          <label className="cursor-pointer rounded-sm text-sm text-text-secondary underline hover:text-text-primary focus-within:outline-none focus-within:ring-2 focus-within:ring-brand-500">
            Upload cityhall.toml
            <input
              type="file"
              accept=".toml,text/plain"
              className="sr-only"
              disabled={busy}
              onChange={(e) => {
                const file = e.target.files?.[0];
                // Clear the input so re-picking the same file still fires.
                e.target.value = "";
                if (file) void upload(file);
              }}
            />
          </label>
          {/* Only offered while there is nothing to lose, and only in the text
              view: the template is entirely comments, which is exactly what the
              form does not keep. */}
          {view === "toml" && !bundle.trim() && (
            <button
              type="button"
              disabled={busy}
              className="cursor-pointer rounded-sm text-sm text-text-secondary underline hover:text-text-primary disabled:cursor-not-allowed disabled:opacity-50"
              onClick={() => edit(BLANK_BUNDLE)}
            >
              Start from scratch
            </button>
          )}
          {config?.updated_at && (
            <span className="text-sm text-text-muted">Last saved {new Date(config.updated_at).toLocaleString()}</span>
          )}
        </div>

        {view === "form" ? (
          doc === null ? (
            <div className="space-y-2 rounded-md border border-surface-700 bg-surface-950 p-4">
              <ErrorText>This document is not valid TOML: {parseError}</ErrorText>
              <p className="text-sm text-text-secondary">
                The form cannot show a document it cannot read. Fix it in the TOML view.
              </p>
            </div>
          ) : (
            <BundleForm doc={doc} disabled={busy} hasComments={hasComments} onChange={editDoc} />
          )
        ) : (
          <textarea
            aria-label="Workspace config bundle"
            disabled={busy}
            value={bundle}
            onChange={(e) => edit(e.target.value)}
            spellCheck={false}
            rows={16}
            placeholder={
              'schema_version = 1\n\n[[projects]]\nname = "my-repo"\nremote = "https://github.com/org/my-repo.git"'
            }
            className="w-full rounded-md border border-surface-700 bg-surface-950 px-3 py-2 font-mono text-xs text-text-primary placeholder:text-text-muted focus:border-brand-500 focus:outline-none focus-visible:ring-2 focus-visible:ring-brand-500 disabled:cursor-not-allowed disabled:opacity-50"
          />
        )}

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
          <Button type="submit" variant="primary" disabled={busy}>
            {saving ? "Saving..." : "Save config"}
          </Button>
        </div>
      </form>
    </>
  );
}

function BundleForm({
  doc,
  disabled,
  hasComments,
  onChange,
}: {
  doc: Doc;
  disabled: boolean;
  hasComments: boolean;
  onChange: (next: Doc) => void;
}) {
  const projects = readProjects(doc);

  function setProjects(rows: ProjectRow[]) {
    onChange(writeProjects(doc, rows));
  }

  function setProject(index: number, patch: Partial<ProjectRow>) {
    setProjects(projects.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  }

  return (
    <div className="space-y-6">
      {hasComments && (
        <p className="text-sm text-status-waiting">
          This document has comments. Editing here rewrites it from its values, which drops them; use the TOML view to
          keep them.
        </p>
      )}

      <div className="space-y-3">
        <h3 className="font-mono text-xs uppercase tracking-wider text-text-muted">Projects</h3>
        <p className="text-sm text-text-secondary">
          Cloned into every workspace on its next start. Addressed by git remote, not by path, because an admin's local
          checkout means nothing in a container. An existing checkout is left alone, so no user loses uncommitted work.
        </p>

        {projects.length === 0 && <p className="text-sm text-text-muted">No projects. Workspaces start empty.</p>}

        {projects.map((row, i) => (
          <div key={i} className="flex flex-wrap items-end gap-2">
            <label className="flex-1 space-y-1">
              <span className="text-xs text-text-muted">Name</span>
              <Input
                value={row.name}
                disabled={disabled}
                placeholder="my-repo"
                onChange={(e) => setProject(i, { name: e.target.value })}
              />
            </label>
            <label className="flex-[2] space-y-1">
              <span className="text-xs text-text-muted">Git remote</span>
              <Input
                value={row.remote}
                disabled={disabled}
                placeholder="https://github.com/org/my-repo.git"
                onChange={(e) => setProject(i, { remote: e.target.value })}
              />
            </label>
            <label className="flex-1 space-y-1">
              <span className="text-xs text-text-muted">Base branch</span>
              <Input
                value={row.default_base_branch}
                disabled={disabled}
                placeholder="detected"
                onChange={(e) => setProject(i, { default_base_branch: e.target.value })}
              />
            </label>
            <Button
              type="button"
              variant="danger"
              disabled={disabled}
              aria-label={`Remove project ${row.name || i + 1}`}
              onClick={() => setProjects(projects.filter((_, j) => j !== i))}
            >
              Remove
            </Button>
          </div>
        ))}

        <Button
          type="button"
          disabled={disabled}
          onClick={() => setProjects([...projects, { name: "", remote: "", default_base_branch: "", extra: {} }])}
        >
          Add project
        </Button>
      </div>

      <div className="space-y-3">
        <h3 className="font-mono text-xs uppercase tracking-wider text-text-muted">Settings</h3>
        <p className="text-sm text-text-secondary">
          The settings worth choosing without an aoe install in front of you. CityHall cannot see which aoe version a
          workspace runs, so it does not try to render every field: anything not here belongs in the TOML view, and the
          Settings &rarr; CityHall page of an aoe install generates a complete document for you.
        </p>

        {FIELDS.map((f) => (
          <SettingRow
            key={`${f.section}.${f.field}`}
            field={f}
            value={readSetting(doc, f)}
            disabled={disabled}
            onChange={(value) => onChange(writeSetting(doc, f, value))}
          />
        ))}
      </div>
    </div>
  );
}

/// One settings leaf. Every control carries an explicit "aoe default" state,
/// because an absent key and a key set to the default are not the same thing:
/// the first follows whatever the workspace's aoe build does, the second pins
/// it. A plain checkbox could not express the difference.
function SettingRow({
  field,
  value,
  disabled,
  onChange,
}: {
  field: FormField;
  value: unknown;
  disabled: boolean;
  onChange: (value: unknown) => void;
}) {
  const control = () => {
    switch (field.kind) {
      case "toggle":
        return (
          <Select
            value={value === undefined ? "" : String(Boolean(value))}
            disabled={disabled}
            onChange={(e) => onChange(e.target.value === "" ? undefined : e.target.value === "true")}
          >
            <option value="">Use aoe default</option>
            <option value="true">On</option>
            <option value="false">Off</option>
          </Select>
        );
      case "select":
        return (
          <Select
            value={value === undefined ? "" : String(value)}
            disabled={disabled}
            onChange={(e) => onChange(e.target.value === "" ? undefined : e.target.value)}
          >
            <option value="">Use aoe default</option>
            {field.options.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </Select>
        );
      case "number":
        return (
          <Input
            type="number"
            min={field.min}
            value={value === undefined ? "" : String(value)}
            disabled={disabled}
            placeholder="aoe default"
            onChange={(e) => onChange(e.target.value === "" ? undefined : Number(e.target.value))}
          />
        );
      case "text":
        return (
          <Input
            value={value === undefined ? "" : String(value)}
            disabled={disabled}
            placeholder={`${field.placeholder} (aoe default)`}
            onChange={(e) => onChange(e.target.value === "" ? undefined : e.target.value)}
          />
        );
    }
  };

  return (
    <label className="flex flex-wrap items-start gap-3 border-t border-surface-800 pt-3">
      <span className="min-w-48 flex-1 space-y-0.5">
        <span className="block text-sm text-text-primary">{field.label}</span>
        <span className="block text-xs text-text-muted">{field.description}</span>
        <code className="block text-xs text-text-muted">
          settings.{field.section}.{field.field}
        </code>
      </span>
      <span className="w-64">{control()}</span>
    </label>
  );
}
