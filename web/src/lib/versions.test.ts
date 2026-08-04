import { describe, expect, it } from "vitest";
import { isOlderVersion } from "./versions";

describe("isOlderVersion", () => {
  it("reports an older patch release", () => {
    expect(isOlderVersion("v1.9.9", "v1.10.0")).toBe(true);
  });

  it("reports a newer release as not older", () => {
    expect(isOlderVersion("v1.10.0", "v1.9.9")).toBe(false);
  });

  it("reports an identical version as not older", () => {
    expect(isOlderVersion("v1.13.2", "v1.13.2")).toBe(false);
  });

  it("compares versions with a differing number of components", () => {
    expect(isOlderVersion("v1.13", "v1.13.1")).toBe(true);
    expect(isOlderVersion("v1.13.0", "v1.13")).toBe(false);
  });

  it("treats a tag without digits as never outdated", () => {
    expect(isOlderVersion("latest", "v1.13.2")).toBe(false);
  });
});
