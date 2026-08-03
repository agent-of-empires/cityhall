/// Structured view over the workspace config bundle (#45).
///
/// The TOML text stays canonical: the form parses it, edits the parsed
/// document, and writes it back. That is what lets the two views switch
/// without a save in between, and it is why a key the form does not render
/// survives an edit, since it is still sitting in the parsed object.
///
/// Comments do not survive, because `stringify` re-emits the document from
/// values alone. The form says so; the TOML view is the place to keep prose.
///
/// The field list below is CityHall's own, not aoe's `GET /api/settings/schema`.
/// CityHall has no aoe process to ask, and the aoe version a workspace runs is
/// per-user, so any schema it cached could describe a version nobody runs. A
/// curated subset plus the raw editor is honest about that; see #45.

import { parse, stringify } from "smol-toml";

export type Doc = Record<string, unknown>;

/// A settings leaf the form renders. `undefined` as a value means "not
/// overridden": the key is absent from the document and the workspace uses
/// whatever the aoe build defaults to.
export type FormField = {
  section: string;
  field: string;
  label: string;
  description: string;
} & (
  | { kind: "text"; placeholder: string }
  | { kind: "toggle" }
  | { kind: "number"; min: number }
  | { kind: "select"; options: { value: string; label: string }[] }
);

/// Labels and descriptions are paraphrased from the `#[setting(...)]`
/// declarations in aoe's `src/session/config.rs`. Keys must match exactly:
/// aoe's `validate_patch` rejects the whole document on an unknown one, and
/// that failure only surfaces at workspace boot.
///
/// Deliberately short. Every entry has to be worth an admin's attention with no
/// aoe install in front of them, and it must not be `local_only` in aoe's
/// schema, because `apply` strips those before merging.
export const FIELDS: FormField[] = [
  {
    section: "acp",
    field: "default_agent",
    label: "Default agent",
    description: "Agent a session uses when none is named, e.g. claude-code, gemini, aoe-agent.",
    kind: "text",
    placeholder: "claude-code",
  },
  {
    section: "acp",
    field: "offer_structured_in_new_session",
    label: "Offer structured view",
    description: "Show the structured-view toggle in the new-session dialog. Off in aoe by default.",
    kind: "toggle",
  },
  {
    section: "session",
    field: "yolo_mode_default",
    label: "YOLO mode by default",
    description: "Start new sessions with the agent's permission prompts skipped.",
    kind: "toggle",
  },
  {
    section: "session",
    field: "smart_rename",
    label: "Smart session rename",
    description: "Name a new session from its first turn instead of leaving the generated title.",
    kind: "toggle",
  },
  {
    section: "worktree",
    field: "enabled",
    label: "Worktree mode by default",
    description: "Create new sessions in their own git worktree rather than in the checkout itself.",
    kind: "toggle",
  },
  {
    section: "worktree",
    field: "auto_cleanup",
    label: "Clean up worktrees",
    description: "Remove a session's worktree when the session is deleted.",
    kind: "toggle",
  },
  {
    section: "updates",
    field: "update_check_mode",
    label: "Update checks",
    description:
      "A workspace is replaced by its image, so a user cannot act on an update banner; `off` suits most deployments.",
    kind: "select",
    options: [
      { value: "notify", label: "Notify" },
      { value: "auto", label: "Install automatically" },
      { value: "off", label: "Off" },
    ],
  },
  {
    section: "acp",
    field: "replay_events",
    label: "History cap (events)",
    description: "Per-session cap on retained agent events, to bound a workspace's disk use. 0 keeps everything.",
    kind: "number",
    min: 0,
  },
];

/// One `[[projects]]` entry, cloned into every workspace on its next start.
/// `extra` carries whatever else the entry held, so an edit here does not drop
/// a key this build does not know about.
export interface ProjectRow {
  name: string;
  remote: string;
  default_base_branch: string;
  extra: Doc;
}

export function parseBundle(text: string): { doc: Doc } | { error: string } {
  if (!text.trim()) return { doc: {} };
  try {
    const doc = parse(text);
    // A top-level array or scalar parses fine as TOML but is not a bundle, and
    // the form would silently treat it as empty.
    if (typeof doc !== "object" || doc === null || Array.isArray(doc)) {
      return { error: "the document is not a TOML table" };
    }
    return { doc: doc as Doc };
  } catch (e) {
    return { error: e instanceof Error ? e.message : "the document is not valid TOML" };
  }
}

export function stringifyBundle(doc: Doc): string {
  // `schema_version` is what the server checks first, and a document written
  // from the form has no other way to acquire it.
  const withVersion = "schema_version" in doc ? doc : { schema_version: 1, ...doc };
  return `${stringify(withVersion)}\n`;
}

function table(doc: Doc, key: string): Doc | undefined {
  const value = doc[key];
  return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Doc) : undefined;
}

export function readSetting(doc: Doc, f: FormField): unknown {
  return table(table(doc, "settings") ?? {}, f.section)?.[f.field];
}

/// Set a settings leaf, or remove it when `value` is `undefined`.
///
/// Removal prunes the section and the `[settings]` table once they are empty,
/// so clearing every override leaves a document that overrides nothing rather
/// than one carrying empty tables that read as configuration.
export function writeSetting(doc: Doc, f: FormField, value: unknown): Doc {
  const settings = { ...(table(doc, "settings") ?? {}) };
  const section = { ...(table(settings, f.section) ?? {}) };

  if (value === undefined) delete section[f.field];
  else section[f.field] = value;

  if (Object.keys(section).length > 0) settings[f.section] = section;
  else delete settings[f.section];

  const next = { ...doc };
  if (Object.keys(settings).length > 0) next.settings = settings;
  else delete next.settings;
  return next;
}

/// How many settings leaves the document overrides, including sections the
/// form does not render.
export function settingsCount(doc: Doc): number {
  const settings = table(doc, "settings") ?? {};
  return Object.keys(settings).reduce((n, key) => n + Object.keys(table(settings, key) ?? {}).length, 0);
}

function rawProjects(doc: Doc): Doc[] {
  const value = doc.projects;
  if (!Array.isArray(value)) return [];
  return value.filter((p): p is Doc => typeof p === "object" && p !== null && !Array.isArray(p));
}

export function readProjects(doc: Doc): ProjectRow[] {
  return rawProjects(doc).map((p) => {
    const { name, remote, default_base_branch, ...extra } = p;
    return {
      name: typeof name === "string" ? name : "",
      remote: typeof remote === "string" ? remote : "",
      default_base_branch: typeof default_base_branch === "string" ? default_base_branch : "",
      extra,
    };
  });
}

export function writeProjects(doc: Doc, rows: ProjectRow[]): Doc {
  const next = { ...doc };
  if (rows.length === 0) {
    delete next.projects;
    return next;
  }
  next.projects = rows.map(({ name, remote, default_base_branch, extra }) => ({
    ...extra,
    name,
    remote,
    // Absent means "use whatever the repo's default branch is", which is not
    // the same as an empty string, and aoe validates it as a non-empty name.
    ...(default_base_branch ? { default_base_branch } : {}),
  }));
  return next;
}
