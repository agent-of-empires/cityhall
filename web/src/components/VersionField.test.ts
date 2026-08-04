import { describe, expect, it } from "vitest";
import { versionMode } from "./VersionField";

const RELEASES = ["v1.13.2", "v1.13.1"];

describe("versionMode", () => {
  it("infers release for a version in the discovered list", () => {
    expect(versionMode("v1.13.2", RELEASES, null)).toBe("release");
  });

  it("infers custom for a tag the list does not have", () => {
    expect(versionMode("main-20260804", RELEASES, null)).toBe("custom");
  });

  // The regression: both props arrive from requests that land after mount, so a
  // value seeded once on mount left a saved custom tag in the release dropdown
  // with the checkbox unticked.
  it("follows the props when they arrive late", () => {
    expect(versionMode("", [], null)).toBe("release");
    expect(versionMode("main-20260804", [], null)).toBe("custom");
    // The list arriving without the value in it does not change the answer.
    expect(versionMode("main-20260804", RELEASES, null)).toBe("custom");
  });

  it("keeps the user's choice once they have made one", () => {
    // Ticked the box, has not typed anything yet: inference would say release.
    expect(versionMode("", RELEASES, "custom")).toBe("custom");
    // Unticked it while a custom tag is still in the field.
    expect(versionMode("main-20260804", RELEASES, "release")).toBe("release");
  });
});
