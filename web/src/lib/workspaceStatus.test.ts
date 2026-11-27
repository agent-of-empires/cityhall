import { describe, expect, it } from "vitest";
import { modeText } from "./workspaceStatus";

describe("modeText", () => {
  it("names each mode with the glossary terms", () => {
    expect(modeText("full", null)).toBe("Full mode");
    expect(modeText("composer", null)).toBe("Composer mode");
  });

  it("appends the reason when the backend refused Full mode", () => {
    expect(modeText("composer", "shared docker network")).toBe("Composer mode (shared docker network)");
  });
});
