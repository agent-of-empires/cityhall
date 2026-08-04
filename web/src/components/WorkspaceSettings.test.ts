import { describe, expect, it } from "vitest";
import { effectiveTelemetryPolicy, telemetryOverrideNote } from "./WorkspaceSettings";

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
});
