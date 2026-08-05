import { useCallback, useEffect, useState } from "react";
import { api, ApiError, can, type GitCredential, type Me } from "../lib/api";
import { AgentCredentialsEditor } from "./AgentCredentials";
import { GitSshKeyEditor } from "./GitSshKey";
import { PageBody, PageHeader } from "./AppShell";
import { Button, Card, ErrorText, Field, Input, Segmented } from "./ui";

/// A user's own settings. Today that is the git credential their workspace uses
/// to clone, pull, and push (#43).
///
/// Self-service and per user rather than one shared deployment credential, so a
/// commit made inside a workspace is attributed to the person who made it and
/// deleting an account revokes exactly their access.
export function AccountPage({ me }: { me: Me }) {
  const canUseWorkspace = can(me, "workspaces.use");

  const [cred, setCred] = useState<GitCredential | null>(null);
  const [host, setHost] = useState("");
  const [username, setUsername] = useState("");
  const [token, setToken] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  // Which of the two git auth methods is showing. Purely a display choice: the
  // token form and GitSshKeyEditor are two independent credentials with their
  // own save/remove calls, so switching tabs never touches the other one.
  const [gitMode, setGitMode] = useState<"token" | "ssh">("token");

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

  const [restarting, setRestarting] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  const [restarted, setRestarted] = useState(false);

  async function restart() {
    if (!confirm("Restart your workspace? Any running agent sessions inside it will end.")) return;
    setRestarting(true);
    setRestartError(null);
    setRestarted(false);
    try {
      await api.restartMyWorkspace();
      setRestarted(true);
    } catch (err) {
      setRestartError(err instanceof ApiError ? err.message : "could not restart your workspace");
    } finally {
      setRestarting(false);
    }
  }

  return (
    <>
      <PageHeader title="Account" />
      <PageBody className="max-w-[660px]">
        {!canUseWorkspace ? (
          <p className="text-sm text-text-dim">
            You do not have a workspace, so there is nowhere for a git credential to be used.
          </p>
        ) : (
          <div className="flex flex-col gap-4">
            {loadError && <ErrorText>{loadError}</ErrorText>}

            <Card>
              <div className="flex flex-col gap-4">
                <div>
                  <div className="text-[14.5px] font-semibold text-text-bright">Git credential</div>
                  <p className="mt-1 text-[13px] leading-relaxed text-text-dim">
                    Used by your workspace to clone, pull, and push. Commits are made as{" "}
                    <span className="text-text-bright">{me.username}</span>
                    {me.email && <> &lt;{me.email}&gt;</>}. A personal access token with repository access is enough, or
                    an SSH key for <span className="font-mono text-text-bright">git@</span> remotes. It is stored
                    encrypted and never shown again.
                  </p>
                </div>
                <Segmented
                  value={gitMode}
                  onChange={setGitMode}
                  className="self-start"
                  options={[
                    { value: "token", label: "Token" },
                    { value: "ssh", label: "SSH key" },
                  ]}
                />

                {gitMode === "token" && (
                  <form onSubmit={save} className="flex flex-col gap-4">
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

                    <p className="text-[12.5px] text-text-hint">
                      A change applies the next time your workspace starts.
                    </p>

                    {saveError && <ErrorText>{saveError}</ErrorText>}
                    {saved && <p className="text-sm text-running">Git credential saved.</p>}

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
                )}
              </div>
            </Card>

            {gitMode === "ssh" && <GitSshKeyEditor />}

            <AgentCredentialsEditor
              load={api.getAgentCredentials}
              update={api.updateAgentCredential}
              remove={api.deleteAgentCredential}
            />

            <Card>
              <div className="flex items-center gap-3">
                <Button disabled={restarting} onClick={() => void restart()}>
                  {restarting ? "Restarting..." : "Restart workspace"}
                </Button>
                <p className="text-[12.5px] text-text-dim">Restarting is what applies any credential changes above.</p>
              </div>
              {restartError && <ErrorText>{restartError}</ErrorText>}
              {restarted && <p className="mt-2 text-sm text-running">Workspace restarted.</p>}
            </Card>

            <Card>
              <div className="flex flex-col gap-2.5">
                <div className="text-[14.5px] font-semibold text-text-bright">Subscriptions, from a terminal</div>
                <p className="text-[13px] leading-relaxed text-text-dim">
                  Codex, Gemini, and OpenCode authenticate through their own interactive login, run in a terminal
                  session inside your workspace. CityHall never sees these: they write to the agent's own config
                  directory, which is kept on your volume, so the login survives a restart.
                </p>
                <div className="rounded-md bg-code-bg px-3.5 py-3 font-mono text-[12.5px] leading-[1.9] text-text">
                  <div>
                    <span className="text-text-faint">$</span> codex login
                  </div>
                  <div>
                    <span className="text-text-faint">$</span> gemini
                  </div>
                  <div>
                    <span className="text-text-faint">$</span> opencode auth login
                  </div>
                </div>
                <p className="text-[12.5px] leading-relaxed text-text-hint">
                  A Claude Pro or Max subscription is different: run{" "}
                  <span className="font-mono text-text-dim">claude setup-token</span> anywhere you are already logged
                  in, and store the result above.
                </p>
              </div>
            </Card>
          </div>
        )}
      </PageBody>
    </>
  );
}
