// Contract test for the form's view of the bundle. The form renders a curated
// subset of aoe's settings, so the thing that has to hold is that editing
// through it never drops what it does not render (#45).

import { describe, expect, it } from "vitest";
import {
  FIELDS,
  parseBundle,
  readProjects,
  readSetting,
  settingsCount,
  stringifyBundle,
  writeProjects,
  writeSetting,
  type Doc,
} from "./bundleForm";

const field = (section: string, name: string) => FIELDS.find((f) => f.section === section && f.field === name)!;

/// A section the form does not render, a `[meta]` block, and a project carrying
/// an extra key: everything an edit could silently discard.
const RICH = `schema_version = 1

[meta]
generated_by = "aoe v1.13.2"

[settings.acp]
default_agent = "claude-code"

[settings.logging]
level = "debug"

[[projects]]
name = "demo"
remote = "https://example.com/demo.git"
some_future_key = 7
`;

function doc(text: string): Doc {
  const parsed = parseBundle(text);
  if ("error" in parsed) throw new Error(parsed.error);
  return parsed.doc;
}

/// Every key must be reachable through the same path aoe validates, so a typo
/// here would only surface as a rejected bundle at workspace boot.
describe("bundleForm", () => {
  it("keeps what it does not render when a rendered field is edited", () => {
    const edited = writeSetting(doc(RICH), field("session", "yolo_mode_default"), true);
    const out = doc(stringifyBundle(edited));

    expect(readSetting(out, field("session", "yolo_mode_default"))).toBe(true);
    // Untouched: a whole section with no form row, and the generated-by block.
    expect((out.settings as Doc).logging).toEqual({ level: "debug" });
    expect(out.meta).toEqual({ generated_by: "aoe v1.13.2" });
    expect(readSetting(out, field("acp", "default_agent"))).toBe("claude-code");
  });

  it("keeps a project key it has no row for", () => {
    const rows = readProjects(doc(RICH));
    rows[0].remote = "https://example.com/renamed.git";
    const out = doc(stringifyBundle(writeProjects(doc(RICH), rows)));

    expect(out.projects).toEqual([{ name: "demo", remote: "https://example.com/renamed.git", some_future_key: 7 }]);
  });

  it("prunes a section and [settings] once the last override is cleared", () => {
    let d = doc('schema_version = 1\n\n[settings.acp]\ndefault_agent = "gemini"\n');
    d = writeSetting(d, field("acp", "default_agent"), undefined);

    expect(d.settings).toBeUndefined();
    expect(settingsCount(d)).toBe(0);
    expect(stringifyBundle(d).trim()).toBe("schema_version = 1");
  });

  it("counts overrides the form does not render", () => {
    // 1 acp + 1 logging: the summary must not read as "1 setting" just because
    // only one of them has a form row.
    expect(settingsCount(doc(RICH))).toBe(2);
  });

  it("writes schema_version into a document that has none", () => {
    const out = stringifyBundle(writeSetting({}, field("acp", "default_agent"), "gemini"));
    expect(doc(out).schema_version).toBe(1);
  });

  it("drops an empty base branch rather than writing one aoe rejects", () => {
    const written = writeProjects({}, [
      { name: "a", remote: "r", default_base_branch: "", extra: {} },
      { name: "b", remote: "r", default_base_branch: "main", extra: {} },
    ]);
    expect(written.projects).toEqual([
      { name: "a", remote: "r" },
      { name: "b", remote: "r", default_base_branch: "main" },
    ]);
  });

  it("reports a document the form cannot edit instead of treating it as empty", () => {
    const cases = ["schema_version = ", "[[projects]\nname = 1", "= 3"];
    for (const bad of cases) {
      expect(parseBundle(bad)).toHaveProperty("error");
    }
    // Valid TOML, but not a table: parses, and would otherwise look blank.
    expect(parseBundle("")).toEqual({ doc: {} });
  });
});
