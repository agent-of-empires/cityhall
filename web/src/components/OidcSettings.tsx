import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type OidcSettings } from "../lib/api";
import { Button, ErrorText, Field, Input } from "./ui";

export function OidcSettingsSection() {
  const [settings, setSettings] = useState<OidcSettings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [enabled, setEnabled] = useState(false);
  const [issuer, setIssuer] = useState("");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [scopes, setScopes] = useState("openid email profile");
  const [allowedDomains, setAllowedDomains] = useState("");

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const apply = useCallback((s: OidcSettings) => {
    setSettings(s);
    setEnabled(s.enabled);
    setIssuer(s.issuer);
    setClientId(s.client_id);
    setScopes(s.scopes);
    setAllowedDomains(s.allowed_domains);
    setClientSecret("");
  }, []);

  const load = useCallback(async () => {
    try {
      apply(await api.getOidcSettings());
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof ApiError ? e.message : "could not load OIDC settings");
    }
  }, [apply]);

  useEffect(() => {
    void load();
  }, [load]);

  const envManaged = settings?.env_managed ?? false;
  const disabled = envManaged;
  const callbackUrl = settings ? `${window.location.origin}${settings.callback_path}` : "";

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      apply(
        await api.updateOidcSettings({
          enabled,
          issuer,
          client_id: clientId,
          client_secret: clientSecret || undefined,
          scopes,
          allowed_domains: allowedDomains,
        }),
      );
      setSaved(true);
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not save OIDC settings");
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      <h2 className="mt-4 font-mono text-xs uppercase tracking-wider text-text-muted">SSO / OpenID Connect</h2>

      {loadError && <ErrorText>{loadError}</ErrorText>}

      {envManaged && (
        <div className="rounded-md border border-surface-700 bg-surface-850 px-4 py-3 text-sm text-text-secondary">
          OIDC is configured through environment variables, so these fields are read-only. Unset the{" "}
          <code className="text-text-primary">OIDC_*</code> variables to manage it here instead.
        </div>
      )}

      {settings && !envManaged && !settings.secret_key_available && (
        <div className="rounded-md border border-status-waiting/40 bg-surface-850 px-4 py-3 text-sm text-status-waiting">
          <code className="text-text-primary">CITYHALL_SECRET_KEY</code> is not set. Set it (a base64-encoded 32-byte
          key) before saving a client secret, or the save will be rejected.
        </div>
      )}

      {settings && (
        <div className="rounded-md border border-surface-700 bg-surface-850 px-4 py-3 text-sm text-text-secondary">
          Register this redirect URI with your identity provider:
          <code className="ml-1 break-all text-text-primary">{callbackUrl}</code>
        </div>
      )}

      <form onSubmit={save} className="space-y-4 rounded-lg border border-surface-700 p-5">
        <label className="flex items-center gap-2 text-sm text-text-primary">
          <input
            type="checkbox"
            checked={enabled}
            disabled={disabled}
            onChange={(e) => setEnabled(e.target.checked)}
            className="h-4 w-4 accent-brand-500"
          />
          Enable SSO login
        </label>

        <Field label="Issuer URL">
          <Input
            value={issuer}
            disabled={disabled}
            onChange={(e) => setIssuer(e.target.value)}
            placeholder="https://accounts.example.com"
          />
        </Field>
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field label="Client ID">
            <Input value={clientId} disabled={disabled} onChange={(e) => setClientId(e.target.value)} />
          </Field>
          <Field label="Client secret">
            <Input
              type="password"
              value={clientSecret}
              disabled={disabled}
              onChange={(e) => setClientSecret(e.target.value)}
              placeholder={settings?.client_secret_set ? "•••••••• (unchanged)" : "(optional for public clients)"}
              autoComplete="new-password"
            />
          </Field>
          <Field label="Scopes">
            <Input
              value={scopes}
              disabled={disabled}
              onChange={(e) => setScopes(e.target.value)}
              placeholder="openid email profile"
            />
          </Field>
          <Field label="Allowed email domains">
            <Input
              value={allowedDomains}
              disabled={disabled}
              onChange={(e) => setAllowedDomains(e.target.value)}
              placeholder="(any) example.com, example.org"
            />
          </Field>
        </div>

        {saveError && <ErrorText>{saveError}</ErrorText>}
        {saved && <p className="text-sm text-status-running">Settings saved.</p>}

        {!envManaged && (
          <div className="flex justify-end">
            <Button type="submit" variant="primary" disabled={saving}>
              {saving ? "Saving..." : "Save settings"}
            </Button>
          </div>
        )}
      </form>
    </>
  );
}
