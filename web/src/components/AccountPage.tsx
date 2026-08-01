import { useCallback, useEffect, useState } from "react";
import { api, ApiError, can, type GitCredential, type Me } from "../lib/api";
import { TopBar } from "./TopBar";
import { Button, ErrorText, Field, Input } from "./ui";

/// A user's own settings. Today that is the git credential their workspace uses
/// to clone, pull, and push (#43).
///
/// Self-service and per user rather than one shared deployment credential, so a
/// commit made inside a workspace is attributed to the person who made it and
/// deleting an account revokes exactly their access.
export function AccountPage({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
  const canUseWorkspace = can(me, "workspaces.use");

  const [cred, setCred] = useState<GitCredential | null>(null);
  const [host, setHost] = useState("");
  const [username, setUsername] = useState("");
  const [token, setToken] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const apply = useCallback((c: GitCredential) => {
    setCred(c);
    setHost(c.host);
    setUsername(c.username);
    // Never seed the token field: the server does not return it.
    setToken("");
  }, []);

  const load = useCallback(async () => {
    if (!canUseWorkspace) return;
    try {
      apply(await api.getGitCredential());
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof ApiError ? e.message : "could not load your git credential");
    }
  }, [apply, canUseWorkspace]);

  useEffect(() => {
    void load();
  }, [load]);

  // Editing while a request is in flight would lose the edit: every response
  // reseeds these fields, so a reply that lands after a keystroke overwrites it
  // and then reports the stale value as saved.
  const busy = saving || cred === null;

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      apply(await api.updateGitCredential({ host, username, token: token || null }));
      setSaved(true);
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not save your git credential");
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!confirm("Remove your stored git credential? Private repos will stop working in your workspace.")) return;
    setSaveError(null);
    setSaved(false);
    try {
      apply(await api.deleteGitCredential());
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not remove your git credential");
    }
  }

  return (
    <div className="flex h-full flex-col">
      <TopBar me={me} onLogout={onLogout} />
      <main className="mx-auto w-full max-w-3xl flex-1 space-y-4 overflow-auto p-6">
        <h2 className="font-mono text-xs uppercase tracking-wider text-text-muted">Git credential</h2>

        {!canUseWorkspace ? (
          <p className="text-sm text-text-secondary">
            You do not have a workspace, so there is nowhere for a git credential to be used.
          </p>
        ) : (
          <>
            {loadError && <ErrorText>{loadError}</ErrorText>}

            <form onSubmit={save} className="space-y-4 rounded-lg border border-surface-700 p-5">
              <p className="text-sm text-text-secondary">
                Used by your workspace to clone, pull, and push. Commits are made as{" "}
                <span className="text-text-primary">{me.username}</span>
                {me.email && <> &lt;{me.email}&gt;</>}. A personal access token with repository access is enough; it is
                stored encrypted and never shown again.
              </p>

              {cred && !cred.secret_key_available && (
                <ErrorText>
                  CITYHALL_SECRET_KEY is not configured on the server, so a credential cannot be stored. Ask an
                  administrator to set it.
                </ErrorText>
              )}

              <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                <Field label="Host">
                  <Input
                    value={host}
                    onChange={(e) => {
                      setHost(e.target.value);
                      setSaved(false);
                    }}
                    placeholder="https://github.com"
                    disabled={busy}
                  />
                </Field>
                <Field label="Username">
                  <Input
                    value={username}
                    onChange={(e) => {
                      setUsername(e.target.value);
                      setSaved(false);
                    }}
                    placeholder="your-username"
                    disabled={busy}
                  />
                </Field>
              </div>
              <Field label={cred?.token_set ? "Token (leave blank to keep the stored one)" : "Token"}>
                <Input
                  type="password"
                  value={token}
                  onChange={(e) => {
                    setToken(e.target.value);
                    setSaved(false);
                  }}
                  autoComplete="new-password"
                  placeholder={cred?.token_set ? "unchanged" : "ghp_..."}
                  disabled={busy}
                />
              </Field>

              <p className="text-sm text-text-secondary">A change applies the next time your workspace starts.</p>

              {saveError && <ErrorText>{saveError}</ErrorText>}
              {saved && <p className="text-sm text-status-running">Git credential saved.</p>}

              <div className="flex justify-between">
                {cred?.token_set ? (
                  <Button type="button" variant="danger" disabled={busy} onClick={() => void remove()}>
                    Remove
                  </Button>
                ) : (
                  <span />
                )}
                <Button type="submit" variant="primary" disabled={busy || !cred?.secret_key_available}>
                  {saving ? "Saving..." : "Save credential"}
                </Button>
              </div>
            </form>
          </>
        )}
      </main>
    </div>
  );
}
