import { describe, expect, it } from "vitest";
import { agentCredentialPlaceholder, canSaveAgentCredential } from "./AgentCredentials";
import type { AgentCredential } from "../lib/api";

const CREDENTIAL: AgentCredential = {
  env_var: "ANTHROPIC_API_KEY",
  label: "Anthropic API key (Claude)",
  structured_view: true,
  limitation: null,
  value_set: false,
  usable: false,
};

// There is no @testing-library/react in this repo, so these cover the two
// pure predicates behind the editor's "blank cannot be submitted" and
// "placeholder never implies the secret itself" behavior without rendering.
describe("canSaveAgentCredential", () => {
  it("rejects an empty value", () => {
    expect(canSaveAgentCredential("")).toBe(false);
  });

  it("rejects a whitespace-only value", () => {
    expect(canSaveAgentCredential("   ")).toBe(false);
  });

  it("accepts a non-empty value", () => {
    expect(canSaveAgentCredential("sk-ant-...")).toBe(true);
  });
});

describe("agentCredentialPlaceholder", () => {
  it("shows an unchanged hint when a value is already stored", () => {
    expect(agentCredentialPlaceholder({ ...CREDENTIAL, value_set: true })).toBe("•••••••• (unchanged)");
  });

  it("shows not-set when nothing is stored", () => {
    expect(agentCredentialPlaceholder({ ...CREDENTIAL, value_set: false })).toBe("(not set)");
  });
});
