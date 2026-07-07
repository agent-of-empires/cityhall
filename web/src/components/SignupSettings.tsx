import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type Role, type SignupSettings } from "../lib/api";
import { Button, ErrorText, Field, Input, Select } from "./ui";

export function SignupSettingsSection() {
  const [loadError, setLoadError] = useState<string | null>(null);
  const [roles, setRoles] = useState<Role[]>([]);

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
    <>
      <h2 className="mt-4 font-mono text-xs uppercase tracking-wider text-text-muted">Self-signup</h2>

      {loadError && <ErrorText>{loadError}</ErrorText>}

      <form onSubmit={save} className="space-y-4 rounded-lg border border-surface-700 p-5">
        <label className="flex items-center gap-2 text-sm text-text-primary">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
            className="h-4 w-4 accent-brand-500"
          />
          Allow public sign-up (requires SMTP for email verification)
        </label>

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
        {saved && <p className="text-sm text-status-running">Settings saved.</p>}

        <div className="flex justify-end">
          <Button type="submit" variant="primary" disabled={saving}>
            {saving ? "Saving..." : "Save settings"}
          </Button>
        </div>
      </form>
    </>
  );
}
