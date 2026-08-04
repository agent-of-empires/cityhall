import { describe, expect, it } from "vitest";
import { canSaveSshKey } from "./GitSshKey";
import type { GitSshKey } from "../lib/api";

const NOTHING_STORED: GitSshKey = { key_set: false, known_hosts: "", secret_key_available: true };
const STORED: GitSshKey = { key_set: true, known_hosts: "github.com ssh-ed25519 AAAA", secret_key_available: true };
const HOSTS = "github.com ssh-ed25519 AAAA";
const KEY = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----";

// There is no @testing-library/react in this repo, so this covers the one
// predicate behind the save control without rendering. The server validates the
// key and the host keys properly; what matters here is that the form cannot
// submit a combination the server is certain to reject.
describe("canSaveSshKey", () => {
  it("refuses to submit before anything has loaded", () => {
    expect(canSaveSshKey(null, KEY, HOSTS)).toBe(false);
  });

  it("refuses a key with no known_hosts, which is the whole point of requiring them", () => {
    expect(canSaveSshKey(NOTHING_STORED, KEY, "")).toBe(false);
    expect(canSaveSshKey(NOTHING_STORED, KEY, "   \n")).toBe(false);
  });

  it("refuses known_hosts with no key when none is stored", () => {
    expect(canSaveSshKey(NOTHING_STORED, "", HOSTS)).toBe(false);
  });

  it("accepts a key with known_hosts", () => {
    expect(canSaveSshKey(NOTHING_STORED, KEY, HOSTS)).toBe(true);
  });

  it("accepts editing known_hosts alone once a key is stored", () => {
    expect(canSaveSshKey(STORED, "", "gitlab.com ssh-ed25519 BBBB")).toBe(true);
  });

  it("refuses everything without a server secret key, since nothing could be stored", () => {
    expect(canSaveSshKey({ ...NOTHING_STORED, secret_key_available: false }, KEY, HOSTS)).toBe(false);
  });
});
