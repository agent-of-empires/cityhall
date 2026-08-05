import { useCallback, useEffect, useState } from "react";
import { AlertTriangle } from "lucide-react";
import { ApiError, type AgentCredential, type AgentCredentials as AgentCredentialsData } from "../lib/api";
import { Banner, Button, Card, ErrorText, Input, SectionLabel } from "./ui";

/// A blank value is never "keep existing" here, unlike the git credential form:
/// the server rejects an empty value, so the save control simply refuses to
/// submit one.
export function canSaveAgentCredential(value: string): boolean {
  return value.trim().length > 0;
}

export function agentCredentialPlaceholder(credential: AgentCredential): string {
  return credential.value_set ? "•••••••• (unchanged)" : "(not set)";
}

/// Editor for the agent credentials forwarded into a workspace (#16). Shared
/// between the self-service account page (`/me/...`) and the admin workspaces
/// page (`/users/{id}/...`) rather than duplicated, since the row markup,
/// per-row error/success state, and the secret-key banner are identical; only
/// which endpoints back the load/update/delete calls differs.
export function AgentCredentialsEditor({
  load,
  update,
  remove,
}: {
  load: () => Promise<AgentCredentialsData>;
  update: (envVar: string, value: string) => Promise<AgentCredentialsData>;
  remove: (envVar: string) => Promise<AgentCredentialsData>;
}) {
  const [data, setData] = useState<AgentCredentialsData | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setData(await load());
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof ApiError ? e.message : "could not load agent credentials");
    }
  }, [load]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <Card className="space-y-4">
      <SectionLabel>Agent credentials</SectionLabel>
      <p className="text-sm text-text-dim">
        Forwarded into the workspace so agents running there can authenticate. A change takes effect the next time the
        workspace restarts.
      </p>

      {loadError && <ErrorText>{loadError}</ErrorText>}

      {data && !data.secret_key_available && (
        <Banner tone="warning">
          <code className="font-semibold">CITYHALL_SECRET_KEY</code> is not set. Set it (a base64-encoded 32-byte key)
          before saving an agent credential, or the save will be rejected.
        </Banner>
      )}

      {data && (
        <div className="space-y-4">
          {data.credentials.map((credential) => (
            <CredentialRow
              key={credential.env_var}
              credential={credential}
              disabled={!data.secret_key_available}
              onSave={async (value) => setData(await update(credential.env_var, value))}
              onDelete={async () => setData(await remove(credential.env_var))}
            />
          ))}
        </div>
      )}
    </Card>
  );
}

function CredentialRow({
  credential,
  disabled,
  onSave,
  onDelete,
}: {
  credential: AgentCredential;
  disabled: boolean;
  onSave: (value: string) => Promise<void>;
  onDelete: () => Promise<void>;
}) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  async function save() {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      await onSave(value);
      // Never re-populate the secret field: the server does not return it.
      setValue("");
      setSaved(true);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not save this credential");
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!confirm(`Remove the stored ${credential.label} credential?`)) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      await onDelete();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not remove this credential");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-2 border-b border-border-soft pb-4 last:border-0 last:pb-0">
      <div className="text-sm text-text">
        {credential.label} <code className="text-xs text-text-hint">{credential.env_var}</code>
      </div>

      {credential.limitation && (
        <p className="flex items-start gap-1.5 text-sm text-waiting">
          <AlertTriangle size={14} className="mt-0.5 shrink-0" />
          {credential.limitation}
        </p>
      )}
      {credential.value_set && !credential.usable && (
        <ErrorText>Stored value cannot be read, it must be re-entered.</ErrorText>
      )}

      <div className="flex items-end gap-2">
        <div className="flex-1">
          <Input
            type="password"
            autoComplete="new-password"
            value={value}
            disabled={disabled || busy}
            onChange={(e) => {
              setValue(e.target.value);
              setSaved(false);
            }}
            placeholder={agentCredentialPlaceholder(credential)}
          />
        </div>
        <Button
          variant="primary"
          disabled={disabled || busy || !canSaveAgentCredential(value)}
          onClick={() => void save()}
        >
          Save
        </Button>
        {credential.value_set && (
          <Button variant="danger" disabled={disabled || busy} onClick={() => void remove()}>
            Delete
          </Button>
        )}
      </div>

      {error && <ErrorText>{error}</ErrorText>}
      {saved && <p className="text-sm text-running">Saved.</p>}
    </div>
  );
}
