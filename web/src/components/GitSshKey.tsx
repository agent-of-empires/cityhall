import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type GitSshKey as GitSshKeyData } from "../lib/api";
import { Button, ErrorText, Field } from "./ui";

/// Whether the save control can submit. A blank key means "keep the stored one",
/// like the git credential form, so it is only missing when nothing is stored;
/// known_hosts is always required, because a key with nothing to verify against
/// is what the workspace refuses to use.
export function canSaveSshKey(cred: GitSshKeyData | null, key: string, knownHosts: string): boolean {
  if (!cred?.secret_key_available) return false;
  if (knownHosts.trim().length === 0) return false;
  return cred.key_set || key.trim().length > 0;
}

const TEXTAREA =
  "w-full rounded-md border border-surface-700 bg-surface-950 px-3 py-2 font-mono text-xs text-text-primary placeholder:text-text-muted focus:border-brand-500 focus:outline-none focus-visible:ring-2 focus-visible:ring-brand-500 disabled:cursor-not-allowed disabled:opacity-50";

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
      <h2 className="font-mono text-xs uppercase tracking-wider text-text-muted">Git SSH key</h2>

      {loadError && <ErrorText>{loadError}</ErrorText>}

      <form onSubmit={save} className="space-y-4 rounded-lg border border-surface-700 p-5">
        <p className="text-sm text-text-secondary">
          For <span className="font-mono text-text-primary">git@host:...</span> remotes, which a token cannot
          authenticate. Stored encrypted and never shown again. The key must have no passphrase: nothing in a workspace
          can prompt for one.
        </p>

        {cred && !cred.secret_key_available && (
          <ErrorText>
            CITYHALL_SECRET_KEY is not configured on the server, so a key cannot be stored. Ask an administrator to set
            it.
          </ErrorText>
        )}

        <Field label={cred?.key_set ? "Private key (leave blank to keep the stored one)" : "Private key"}>
          <textarea
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
            className={TEXTAREA}
          />
        </Field>

        <Field label="Known hosts">
          <textarea
            value={knownHosts}
            onChange={(e) => {
              setKnownHosts(e.target.value);
              setSaved(false);
            }}
            spellCheck={false}
            rows={4}
            disabled={busy}
            placeholder="github.com ssh-ed25519 AAAAC3Nza..."
            className={TEXTAREA}
          />
        </Field>
        <p className="text-sm text-text-secondary">
          Required. Your workspace verifies the host against these and refuses to connect to anything else, so a key on
          its own is not enough. Get them with{" "}
          <span className="font-mono text-text-primary">ssh-keyscan github.com</span> on a machine you trust, and paste
          the output as it comes.
        </p>

        <p className="text-sm text-text-secondary">A change applies the next time your workspace starts.</p>

        {saveError && <ErrorText>{saveError}</ErrorText>}
        {saved && <p className="text-sm text-status-running">SSH key saved.</p>}

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
