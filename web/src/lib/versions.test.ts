import { describe, expect, it } from "vitest";
import { formatVersion, isGitVersion, isOlderVersion } from "./versions";

describe("isGitVersion", () => {
  it("recognizes a source build", () => {
    expect(isGitVersion("git-abc123def0")).toBe(true);
  });

  it("does not treat a release tag as a source build", () => {
    expect(isGitVersion("v1.13.2")).toBe(false);
  });
});

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

  it("never reports a source build as outdated against a release", () => {
    expect(isOlderVersion("git-abc123def0abc123def0abc123def0abc123de", "v1.13.2")).toBe(false);
  });

  it("never reports a release as outdated against a source build", () => {
    expect(isOlderVersion("v1.13.2", "git-abc123def0abc123def0abc123def0abc123de")).toBe(false);
  });

  it("never compares two source builds against each other", () => {
    expect(
      isOlderVersion("git-abc123def0abc123def0abc123def0abc123de", "git-999999999999999999999999999999999999999"),
    ).toBe(false);
  });
});

describe("formatVersion", () => {
  it("shortens a source build to its first 12 sha characters", () => {
    expect(formatVersion("git-abc123def0abc123def0abc123def0abc123de")).toBe("commit abc123def0ab");
  });

  it("leaves a release tag unchanged", () => {
    expect(formatVersion("v1.13.2")).toBe("v1.13.2");
  });
});
