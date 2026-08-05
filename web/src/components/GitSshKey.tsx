import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type GitSshKey as GitSshKeyData } from "../lib/api";
import { Button, ErrorText, Field, SectionLabel, Textarea } from "./ui";

/// Whether the save control can submit. A blank key means "keep the stored one",
/// like the git credential form, so it is only missing when nothing is stored;
/// known_hosts is always required, because a key with nothing to verify against
/// is what the workspace refuses to use.
export function canSaveSshKey(cred: GitSshKeyData | null, key: string, knownHosts: string): boolean {
  if (!cred?.secret_key_available) return false;
  if (knownHosts.trim().length === 0) return false;
  return cred.key_set || key.trim().length > 0;
}

/// GitHub publishes its current SSH host keys here, over HTTPS against a
/// certificate for a name an attacker on the path cannot produce.
const GITHUB_META = "https://api.github.com/meta";

/// GitHub's published host keys as known_hosts lines.
///
/// The endpoint returns bare `<type> <key>` pairs with no host in front of them,
/// which known_hosts needs, so each one is prefixed. Filtering blanks matters
/// because a line with nothing but the host would fail the server's check and
/// report as the user's mistake.
export function githubKnownHosts(sshKeys: string[]): string {
  return sshKeys
    .map((k) => k.trim())
    .filter((k) => k.length > 0)
    .map((k) => `github.com ${k}`)
    .join("\n");
}

/// Fetch GitHub's published host keys.
///
/// Worth a network call rather than telling the user to run `ssh-keyscan`: a scan
/// trusts whatever answers on port 22, so an attacker on the path at that moment
/// supplies the keys the workspace then pins forever, and strict host key
/// checking cannot tell the difference. This endpoint is authenticated by TLS for
/// a name that attacker cannot present. Fetched from the browser, not the server:
/// GitHub sends a permissive CORS header, so a backend route would add a hop and
/// a failure mode without changing what is trusted.
async function fetchGithubKnownHosts(): Promise<string> {
  const res = await fetch(GITHUB_META, { headers: { Accept: "application/vnd.github+json" } });
  if (!res.ok) throw new Error(`GitHub returned ${res.status}`);
  const body: unknown = await res.json();
  const keys = (body as { ssh_keys?: unknown }).ssh_keys;
  if (!Array.isArray(keys) || keys.length === 0) throw new Error("GitHub returned no host keys");
  return githubKnownHosts(keys.filter((k): k is string => typeof k === "string"));
}

/// A user's SSH key for git, for the workspace remotes an HTTPS token cannot
/// reach (#52).
///
/// Its own section rather than more fields on the git credential form: the two
/// are independent, so removing a token must not disturb a key, and the
/// known_hosts requirement only applies here.
export function GitSshKeyEditor() {
  const [cred, setCred] = useState<GitSshKeyData | null>(null);
  const [key, setKey] = useState("");
  const [knownHosts, setKnownHosts] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const [filling, setFilling] = useState(false);
  const [fillError, setFillError] = useState<string | null>(null);

  async function fillFromGithub() {
    setFilling(true);
    setFillError(null);
    setSaved(false);
    try {
      setKnownHosts(await fetchGithubKnownHosts());
    } catch (err) {
      // Named rather than swallowed: the manual path still works, and a user who
      // cannot reach GitHub from their browser needs to know that is why.
      setFillError(
        `could not reach GitHub to fetch its host keys (${err instanceof Error ? err.message : "unknown error"}). Paste them yourself instead.`,
      );
    } finally {
      setFilling(false);
    }
  }

  const apply = useCallback((c: GitSshKeyData) => {
    setCred(c);
    setKnownHosts(c.known_hosts);
    // Never seed the key field: the server does not return it.
    setKey("");
  }, []);

  const load = useCallback(async () => {
    try {
      apply(await api.getGitSshKey());
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof ApiError ? e.message : "could not load your SSH key");
    }
  }, [apply]);

  useEffect(() => {
    void load();
  }, [load]);

  // Every response reseeds these fields, so editing while a request is in flight
  // would lose the edit.
  const busy = saving || cred === null;

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      apply(await api.updateGitSshKey({ key: key || null, known_hosts: knownHosts }));
      setSaved(true);
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not save your SSH key");
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!confirm("Remove your stored SSH key? Remotes that use it will stop working in your workspace.")) return;
    setSaveError(null);
    setSaved(false);
    try {
      apply(await api.deleteGitSshKey());
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not remove your SSH key");
    }
  }

  return (
    <>
      <SectionLabel>Git SSH key</SectionLabel>

      {loadError && <ErrorText>{loadError}</ErrorText>}

      {/* A `<form>`, not `Card`: the primitive only renders a `div`, and this
          section needs real form submit semantics. Styled to match it exactly. */}
      <form onSubmit={save} className="space-y-4 rounded-card border border-border-soft bg-surface px-[22px] py-[18px]">
        <p className="text-sm text-text-dim">
          For <span className="font-mono text-text">git@host:...</span> remotes, which a token cannot authenticate.
          Stored encrypted and never shown again. The key must have no passphrase: nothing in a workspace can prompt for
          one.
        </p>

        {cred && !cred.secret_key_available && (
          <ErrorText>
            CITYHALL_SECRET_KEY is not configured on the server, so a key cannot be stored. Ask an administrator to set
            it.
          </ErrorText>
        )}

        <Field label={cred?.key_set ? "Private key (leave blank to keep the stored one)" : "Private key"}>
          <Textarea
            value={key}
            onChange={(e) => {
              setKey(e.target.value);
              setSaved(false);
            }}
            spellCheck={false}
            rows={6}
            disabled={busy}
            placeholder={
              cred?.key_set
                ? "unchanged"
                : "-----BEGIN OPENSSH PRIVATE KEY-----\n...\n-----END OPENSSH PRIVATE KEY-----"
            }
            className="font-mono text-xs"
          />
        </Field>

        <Field label="Known hosts">
          <Textarea
            value={knownHosts}
            onChange={(e) => {
              setKnownHosts(e.target.value);
              setSaved(false);
            }}
            spellCheck={false}
            rows={4}
            disabled={busy || filling}
            placeholder="github.com ssh-ed25519 AAAAC3Nza..."
            className="font-mono text-xs"
          />
        </Field>
        <div className="flex items-center gap-3">
          <Button type="button" variant="default" disabled={busy || filling} onClick={() => void fillFromGithub()}>
            {filling ? "Fetching..." : "Fill from GitHub"}
          </Button>
          <p className="text-sm text-text-dim">
            Fetches GitHub's published host keys. For another host, run{" "}
            <span className="font-mono text-text">ssh-keyscan &lt;host&gt;</span> somewhere you trust the network, and
            check the result against the fingerprints that host publishes: a scan trusts whatever answers, so pasting
            one unchecked pins whatever was listening.
          </p>
        </div>
        {fillError && <ErrorText>{fillError}</ErrorText>}
        <p className="text-sm text-text-dim">
          Required. Your workspace verifies the host against these and refuses to connect to anything else, so a key on
          its own is not enough.
        </p>

        <p className="text-sm text-text-dim">A change applies the next time your workspace starts.</p>

        {saveError && <ErrorText>{saveError}</ErrorText>}
        {saved && <p className="text-sm text-running">SSH key saved.</p>}

        <div className="flex justify-between">
          {cred?.key_set ? (
            <Button type="button" variant="danger" disabled={busy} onClick={() => void remove()}>
              Remove
            </Button>
          ) : (
            <span />
          )}
          <Button type="submit" variant="primary" disabled={busy || !canSaveSshKey(cred, key, knownHosts)}>
            {saving ? "Saving..." : "Save SSH key"}
          </Button>
        </div>
      </form>
    </>
  );
}
