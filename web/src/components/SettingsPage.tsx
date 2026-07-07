import { useCallback, useEffect, useState } from "react";
import { Send } from "lucide-react";
import { api, ApiError, type Me, type SmtpSettings } from "../lib/api";
import { TopBar } from "./TopBar";
import { OidcSettingsSection } from "./OidcSettings";
import { Button, ErrorText, Field, Input, Select } from "./ui";

export function SettingsPage({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
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
    <div className="flex h-full flex-col">
      <TopBar me={me} onLogout={onLogout} />
      <main className="mx-auto w-full max-w-3xl flex-1 space-y-4 overflow-auto p-6">
        <h2 className="font-mono text-xs uppercase tracking-wider text-text-muted">SMTP / Email</h2>

        {loadError && <ErrorText>{loadError}</ErrorText>}

        {envManaged && (
          <div className="rounded-md border border-surface-700 bg-surface-850 px-4 py-3 text-sm text-text-secondary">
            SMTP is configured through environment variables, so these fields are read-only. Unset the{" "}
            <code className="text-text-primary">SMTP_*</code> variables to manage it here instead.
          </div>
        )}

        {settings && !envManaged && !settings.secret_key_available && (
          <div className="rounded-md border border-status-waiting/40 bg-surface-850 px-4 py-3 text-sm text-status-waiting">
            <code className="text-text-primary">CITYHALL_SECRET_KEY</code> is not set. Set it (a base64-encoded 32-byte
            key) before saving a password, or the save will be rejected.
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
            Enable email sending
          </label>

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
          {saved && <p className="text-sm text-status-running">Settings saved.</p>}

          {!envManaged && (
            <div className="flex justify-end">
              <Button type="submit" variant="primary" disabled={saving}>
                {saving ? "Saving..." : "Save settings"}
              </Button>
            </div>
          )}
        </form>

        <div className="space-y-3 rounded-lg border border-surface-700 p-5">
          <h3 className="font-mono text-xs uppercase tracking-wider text-text-muted">Send test email</h3>
          <p className="text-sm text-text-secondary">Sends a test message using the currently active configuration.</p>
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
            <Button
              variant="default"
              onClick={sendTest}
              disabled={testing || !testTo}
              className="flex items-center gap-1.5"
            >
              <Send size={14} />
              {testing ? "Sending..." : "Send test"}
            </Button>
          </div>
          {testResult && (
            <p className={testResult.ok ? "text-sm text-status-running" : "text-sm text-status-error"}>
              {testResult.message}
            </p>
          )}
        </div>

        <OidcSettingsSection />
      </main>
    </div>
  );
}
