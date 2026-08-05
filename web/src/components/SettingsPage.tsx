import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Send } from "lucide-react";
import { Navigate, useParams } from "react-router-dom";
import { api, ApiError, type SmtpSettings } from "../lib/api";
import { DEFAULT_SETTINGS_TAB, SETTINGS_TABS, settingsTabLabel } from "../lib/settingsTabs";
import { PageBody, PageHeader } from "./AppShell";
import { OidcSettingsSection } from "./OidcSettings";
import { SignupSettingsSection } from "./SignupSettings";
import { WorkspaceSettingsSection } from "./WorkspaceSettings";
import { WorkspaceConfigSection } from "./WorkspaceConfig";
import { Banner, Button, Card, ErrorText, Field, Input, Select, Toggle } from "./ui";

/// One "label + help on the left, control on the right" settings row, matching
/// the mockup's c-setting pattern. Private to this file and the sibling
/// settings sections; each keeps its own copy rather than importing one
/// another's, since Settings*.tsx already import from here and importing back
/// would make a cycle.
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

function SmtpSettingsSection() {
  const [settings, setSettings] = useState<SmtpSettings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Editable form state, seeded from the loaded settings.
  const [enabled, setEnabled] = useState(false);
  const [host, setHost] = useState("");
  const [port, setPort] = useState(587);
  const [encryption, setEncryption] = useState("starttls");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [fromAddress, setFromAddress] = useState("");
  const [fromName, setFromName] = useState("");

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const [testTo, setTestTo] = useState("");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  const apply = useCallback((s: SmtpSettings) => {
    setSettings(s);
    setEnabled(s.enabled);
    setHost(s.host);
    setPort(s.port);
    setEncryption(s.encryption);
    setUsername(s.username ?? "");
    setFromAddress(s.from_address);
    setFromName(s.from_name ?? "");
    setPassword("");
  }, []);

  const load = useCallback(async () => {
    try {
      apply(await api.getSmtpSettings());
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof ApiError ? e.message : "could not load settings");
    }
  }, [apply]);

  useEffect(() => {
    void load();
  }, [load]);

  const envManaged = settings?.env_managed ?? false;
  const disabled = envManaged;

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      const updated = await api.updateSmtpSettings({
        host,
        port,
        encryption,
        username: username || null,
        password: password || undefined,
        from_address: fromAddress,
        from_name: fromName || null,
        enabled,
      });
      apply(updated);
      setSaved(true);
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not save settings");
    } finally {
      setSaving(false);
    }
  }

  async function sendTest() {
    setTesting(true);
    setTestResult(null);
    try {
      const res = await api.testSmtp(testTo);
      setTestResult(
        res.ok
          ? { ok: true, message: `Test email sent to ${testTo}.` }
          : { ok: false, message: res.error ?? "send failed" },
      );
    } catch (err) {
      setTestResult({
        ok: false,
        message: err instanceof ApiError ? err.message : "could not send test email",
      });
    } finally {
      setTesting(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      {loadError && <ErrorText>{loadError}</ErrorText>}

      {envManaged && (
        <Banner>
          SMTP is configured through environment variables, so these fields are read-only. Unset the{" "}
          <code className="text-text">SMTP_*</code> variables to manage it here instead.
        </Banner>
      )}

      {settings && !envManaged && !settings.secret_key_available && (
        <Banner>
          <span className="text-waiting">
            <code className="text-text">CITYHALL_SECRET_KEY</code> is not set. Set it (a base64-encoded 32-byte key)
            before saving a password, or the save will be rejected.
          </span>
        </Banner>
      )}

      <Card>
        <form onSubmit={save} className="flex flex-col gap-4">
          <SettingRow label="Email sending" help="Send account and notification email through this server.">
            <Toggle checked={enabled} onChange={setEnabled} label="Enable email sending" disabled={disabled} />
          </SettingRow>

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field label="Host">
              <Input
                value={host}
                disabled={disabled}
                onChange={(e) => setHost(e.target.value)}
                placeholder="smtp.example.com"
              />
            </Field>
            <Field label="Port">
              <Input
                type="number"
                value={port}
                disabled={disabled}
                onChange={(e) => setPort(Number(e.target.value))}
                min={1}
                max={65535}
              />
            </Field>
            <Field label="Encryption">
              <Select value={encryption} disabled={disabled} onChange={(e) => setEncryption(e.target.value)}>
                <option value="none">None (port 25)</option>
                <option value="starttls">STARTTLS (port 587)</option>
                <option value="tls">TLS/SSL (port 465)</option>
              </Select>
            </Field>
            <Field label="Username">
              <Input
                value={username}
                disabled={disabled}
                onChange={(e) => setUsername(e.target.value)}
                placeholder="(optional)"
                autoComplete="off"
              />
            </Field>
            <Field label="Password">
              <Input
                type="password"
                value={password}
                disabled={disabled}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={settings?.password_set ? "•••••••• (unchanged)" : "(optional)"}
                autoComplete="new-password"
              />
            </Field>
            <Field label="From address">
              <Input
                type="email"
                value={fromAddress}
                disabled={disabled}
                onChange={(e) => setFromAddress(e.target.value)}
                placeholder="cityhall@example.com"
              />
            </Field>
            <Field label="From name">
              <Input
                value={fromName}
                disabled={disabled}
                onChange={(e) => setFromName(e.target.value)}
                placeholder="(optional) CityHall"
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

      <Card>
        <div className="flex flex-col gap-3">
          <div>
            <div className="text-sm font-medium text-text-bright">Send a test message</div>
            <p className="mt-1 text-[12.5px] text-text-dim">Uses the configuration that is active right now.</p>
          </div>
          <div className="flex items-end gap-3">
            <div className="flex-1">
              <Field label="Recipient">
                <Input
                  type="email"
                  value={testTo}
                  onChange={(e) => setTestTo(e.target.value)}
                  placeholder="you@example.com"
                />
              </Field>
            </div>
            <Button onClick={sendTest} disabled={testing || !testTo} className="flex items-center gap-1.5">
              <Send size={14} />
              {testing ? "Sending..." : "Send test"}
            </Button>
          </div>
          {testResult && (
            <p className={testResult.ok ? "text-sm text-running" : "text-sm text-error"}>{testResult.message}</p>
          )}
        </div>
      </Card>
    </div>
  );
}

/// Read-only notes for the Advanced tab: the workspace backend is a deploy-time
/// choice with no settings endpoint, and the secret key note reuses whichever
/// signal is already on hand (SMTP's) rather than inventing an endpoint just to
/// report one boolean.
function AdvancedNotes() {
  const [secretKeyAvailable, setSecretKeyAvailable] = useState<boolean | null>(null);

  useEffect(() => {
    api
      .getSmtpSettings()
      .then((s) => setSecretKeyAvailable(s.secret_key_available))
      .catch(() => {});
  }, []);

  return (
    <Card>
      <div className="flex flex-col gap-3">
        <div>
          <div className="text-sm font-medium text-text-bright">Workspace backend</div>
          <p className="mt-1 text-[12.5px] leading-relaxed text-text-dim">
            Docker, Kubernetes, or a plain process, chosen at deploy time for this CityHall install and not changeable
            here.
          </p>
        </div>
        <div className="border-t border-border-soft pt-3">
          <div className="text-sm font-medium text-text-bright">Secret key</div>
          <p className="mt-1 text-[12.5px] leading-relaxed text-text-dim">
            Git tokens and agent credentials are encrypted with <code className="text-text">CITYHALL_SECRET_KEY</code>.
            Rotating it without the rotation procedure leaves stored credentials unreadable, so accounts then show them
            as needing re-entry.
          </p>
          {secretKeyAvailable !== null && (
            <p className="mt-1.5 text-[12.5px]">
              {secretKeyAvailable ? (
                <span className="text-running">A secret key is configured.</span>
              ) : (
                <span className="text-waiting">No secret key is configured, so nothing above can store a secret.</span>
              )}
            </p>
          )}
        </div>
      </div>
    </Card>
  );
}

/// Settings is one page per tab (#61): the sidebar renders the tab list, this
/// renders whichever tab's sections `useParams` names.
/// Takes no `me`: the route gates on settings.read, and nothing here varies by
/// who is signed in.
export function SettingsPage() {
  const { tab } = useParams<{ tab: string }>();

  if (!SETTINGS_TABS.some((t) => t.slug === tab)) {
    return <Navigate to={`/settings/${DEFAULT_SETTINGS_TAB}`} replace />;
  }

  return (
    <>
      <PageHeader title={settingsTabLabel(tab)} meta="Settings" />
      <PageBody className="max-w-[660px]">
        {tab === "access" && <OidcSettingsSection />}
        {tab === "signup" && <SignupSettingsSection />}
        {tab === "email" && <SmtpSettingsSection />}
        {tab === "workspace-defaults" && <WorkspaceSettingsSection section="defaults" />}
        {tab === "workspace-config" && <WorkspaceConfigSection />}
        {tab === "advanced" && (
          <div className="flex flex-col gap-4">
            <WorkspaceSettingsSection section="advanced" />
            <AdvancedNotes />
          </div>
        )}
      </PageBody>
    </>
  );
}
