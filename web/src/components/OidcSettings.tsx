import { useCallback, useEffect, useState, type ReactNode } from "react";
import { api, ApiError, type OidcSettings } from "../lib/api";
import { Banner, Button, Card, ErrorText, Field, Input, Toggle } from "./ui";

/// One "label + help on the left, control on the right" settings row, matching
/// the mockup's c-setting pattern. Kept local rather than shared from
/// SettingsPage.tsx, which imports this section and would make a cycle.
function SettingRow({ label, help, children }: { label: string; help?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-6">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-accent">{label}</div>
        {help && <p className="mt-1 max-w-[420px] text-[12.5px] leading-relaxed text-text-dim">{help}</p>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

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
    <div className="flex flex-col gap-4">
      {loadError && <ErrorText>{loadError}</ErrorText>}

      {envManaged && (
        <Banner>
          OIDC is configured through environment variables, so these fields are read-only. Unset the{" "}
          <code className="text-text">OIDC_*</code> variables to manage it here instead.
        </Banner>
      )}

      {settings && !envManaged && !settings.secret_key_available && (
        <Banner>
          <span className="text-waiting">
            <code className="text-text">CITYHALL_SECRET_KEY</code> is not set. Set it (a base64-encoded 32-byte key)
            before saving a client secret, or the save will be rejected.
          </span>
        </Banner>
      )}

      {settings && (
        <Banner>
          Register this redirect URI with your identity provider:{" "}
          <code className="break-all text-text">{callbackUrl}</code>
        </Banner>
      )}

      <Card>
        <form onSubmit={save} className="flex flex-col gap-4">
          <SettingRow label="Single sign-on" help="OpenID Connect.">
            <Toggle checked={enabled} onChange={setEnabled} label="Enable SSO login" disabled={disabled} />
          </SettingRow>

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field label="Issuer URL">
              <Input
                value={issuer}
                disabled={disabled}
                onChange={(e) => setIssuer(e.target.value)}
                placeholder="https://accounts.example.com"
              />
            </Field>
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
          {saved && <p className="text-sm text-running">Settings saved.</p>}

          {!envManaged && (
            <div className="flex justify-end">
              <Button type="submit" variant="primary" disabled={saving}>
                {saving ? "Saving..." : "Save settings"}
              </Button>
            </div>
          )}
        </form>
      </Card>
    </div>
  );
}
