import { describe, expect, it } from "vitest";
import {
  effectiveTelemetryPolicy,
  isKnownTelemetryPolicy,
  telemetryOverrideNote,
  toggleAgent,
} from "./WorkspaceSettings";
import type { AvailableAgent } from "../lib/api";

const CATALOG: AvailableAgent[] = [
  { name: "claude", label: "Claude" },
  { name: "codex", label: "Codex" },
  { name: "gemini", label: "Gemini" },
  { name: "opencode", label: "OpenCode" },
];

// There is no @testing-library/react in this repo, so this covers the one
// non-obvious predicate behind the settings form: an environment-pinned policy
// has to be announced, and it must not read as "your save took effect".
describe("telemetryOverrideNote", () => {
  it("says nothing when no override is set", () => {
    expect(telemetryOverrideNote(null)).toBeNull();
  });

  it("names the variable and the policy it pins", () => {
    const note = telemetryOverrideNote("force_off");
    expect(note).toContain("WORKSPACE_TELEMETRY_POLICY");
    expect(note).toContain("Off for everyone");
  });

  it("says a save is stored rather than applied while pinned", () => {
    expect(telemetryOverrideNote("force_on")).toContain("does not apply while it is set");
  });
});

// What gates the force-on disclosure. Reading the selected policy alone would
// hide it when the environment forces telemetry on, and show it falsely when the
// environment forces it off over a stored force-on.
describe("effectiveTelemetryPolicy", () => {
  it("is the selected policy when nothing is pinned", () => {
    expect(effectiveTelemetryPolicy("force_on", null)).toBe("force_on");
    expect(effectiveTelemetryPolicy("user_choice", null)).toBe("user_choice");
  });

  it("lets the override win", () => {
    expect(effectiveTelemetryPolicy("user_choice", "force_on")).toBe("force_on");
    expect(effectiveTelemetryPolicy("force_on", "force_off")).toBe("force_off");
  });

  // A policy stored by a newer CityHall round-trips through the form untouched,
  // but this build enforces nothing for it, exactly as the server reads it.
  it("treats a policy it does not know as user_choice", () => {
    expect(effectiveTelemetryPolicy("force_maybe", null)).toBe("user_choice");
    expect(isKnownTelemetryPolicy("force_maybe")).toBe(false);
    expect(isKnownTelemetryPolicy("force_off")).toBe(true);
  });
});

// There is no @testing-library/react in this repo, so this covers the one piece
// of logic behind the checkbox list rather than rendering it.
describe("toggleAgent", () => {
  it("adds an agent", () => {
    expect(toggleAgent([], CATALOG, "codex", true)).toEqual(["codex"]);
  });

  it("removes an agent", () => {
    expect(toggleAgent(["claude", "codex"], CATALOG, "claude", false)).toEqual(["codex"]);
  });

  // The server stores a canonical set, so a selection sent in a different order
  // would read as a change and recreate every workspace for nothing.
  it("keeps the selection in catalog order however it was built", () => {
    let selected = toggleAgent([], CATALOG, "opencode", true);
    selected = toggleAgent(selected, CATALOG, "claude", true);
    selected = toggleAgent(selected, CATALOG, "gemini", true);
    expect(selected).toEqual(["claude", "gemini", "opencode"]);
  });

  it("does not duplicate an agent that is already selected", () => {
    expect(toggleAgent(["claude"], CATALOG, "claude", true)).toEqual(["claude"]);
  });

  it("unticking the last one clears the selection", () => {
    expect(toggleAgent(["claude"], CATALOG, "claude", false)).toEqual([]);
  });
});
