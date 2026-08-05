import { useCallback, useEffect, useState, type ReactNode } from "react";
import { api, ApiError, type Role, type SignupSettings } from "../lib/api";
import { Banner, Button, Card, ErrorText, Field, Input, Select, Toggle } from "./ui";

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

export function SignupSettingsSection() {
  const [loadError, setLoadError] = useState<string | null>(null);
  const [roles, setRoles] = useState<Role[]>([]);
  const [smtpReady, setSmtpReady] = useState(true);

  const [enabled, setEnabled] = useState(false);
  const [allowedDomains, setAllowedDomains] = useState("");
  const [defaultRoleId, setDefaultRoleId] = useState<number | null>(null);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const apply = useCallback((s: SignupSettings) => {
    setEnabled(s.signup_enabled);
    setAllowedDomains(s.signup_allowed_domains);
    setDefaultRoleId(s.signup_default_role_id);
  }, []);

  const load = useCallback(async () => {
    try {
      apply(await api.getSignupSettings());
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof ApiError ? e.message : "could not load signup settings");
    }
    api
      .listRoles()
      .then(setRoles)
      .catch(() => {});
    api
      .getSmtpSettings()
      .then((s) => setSmtpReady(s.env_managed || s.enabled))
      .catch(() => {});
  }, [apply]);

  useEffect(() => {
    void load();
  }, [load]);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      apply(
        await api.updateSignupSettings({
          signup_enabled: enabled,
          signup_allowed_domains: allowedDomains,
          signup_default_role_id: defaultRoleId,
        }),
      );
      setSaved(true);
    } catch (err) {
      setSaveError(err instanceof ApiError ? err.message : "could not save signup settings");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      {loadError && <ErrorText>{loadError}</ErrorText>}

      {!smtpReady && (
        <Banner>
          <span className="text-waiting">
            SMTP is not configured, so self-signup cannot be enabled. Set up email first.
          </span>
        </Banner>
      )}

      <Card>
        <form onSubmit={save} className="flex flex-col gap-4">
          <SettingRow label="Public sign-up" help="Also lets a new user sign in through SSO for the first time.">
            <Toggle
              checked={enabled}
              onChange={setEnabled}
              label="Allow public sign-up"
              disabled={!smtpReady && !enabled}
            />
          </SettingRow>

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field label="Allowed email domains">
              <Input
                value={allowedDomains}
                onChange={(e) => setAllowedDomains(e.target.value)}
                placeholder="(any) example.com, example.org"
              />
            </Field>
            <Field label="Default role for new sign-ups">
              <Select
                value={defaultRoleId ?? ""}
                onChange={(e) => setDefaultRoleId(e.target.value ? Number(e.target.value) : null)}
              >
                <option value="">member (default)</option>
                {roles.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.name}
                  </option>
                ))}
              </Select>
            </Field>
          </div>

          {saveError && <ErrorText>{saveError}</ErrorText>}
          {saved && <p className="text-sm text-running">Settings saved.</p>}

          <div className="flex justify-end">
            <Button type="submit" variant="primary" disabled={saving}>
              {saving ? "Saving..." : "Save settings"}
            </Button>
          </div>
        </form>
      </Card>
    </div>
  );
}
