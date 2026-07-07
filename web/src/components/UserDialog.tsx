import { useEffect, useState } from "react";
import { api, ApiError, can, type Me, type Role, type User } from "../lib/api";
import { Button, ErrorText, Field, Input, Select } from "./ui";

type Result = { kind: "password"; value: string } | { kind: "email"; address: string };

export function UserDialog({
  me,
  user,
  onClose,
  onSaved,
}: {
  me: Me;
  user: User | null; // null = create mode
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const editing = user !== null;
  const canManageRoles = can(me, "roles.read");
  const [username, setUsername] = useState(user?.username ?? "");
  const [email, setEmail] = useState(user?.email ?? "");
  const [password, setPassword] = useState("");
  const [sendSetup, setSendSetup] = useState(false);
  const [roles, setRoles] = useState<Role[]>([]);
  const [roleId, setRoleId] = useState<number | null>(user?.role_id ?? null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Result | null>(null);

  useEffect(() => {
    if (!canManageRoles) return;
    api
      .listRoles()
      .then((rs) => {
        setRoles(rs);
        // Default a new user to the `member` role when present.
        if (!editing && roleId === null) {
          setRoleId(rs.find((r) => r.name === "member")?.id ?? rs[0]?.id ?? null);
        }
      })
      .catch(() => {});
  }, [canManageRoles, editing, roleId]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      if (editing) {
        await api.updateUser(user.id, {
          username,
          email,
          ...(password ? { password } : {}),
          ...(canManageRoles && roleId !== null ? { role_id: roleId } : {}),
        });
        await onSaved();
        onClose();
        return;
      }
      const res = await api.createUser({
        username,
        email: email || null,
        password: sendSetup ? undefined : password || undefined,
        sendSetupEmail: sendSetup,
        roleId: canManageRoles && roleId !== null ? roleId : undefined,
      });
      await onSaved();
      if (res.generated_password) {
        setResult({ kind: "password", value: res.generated_password });
      } else if (sendSetup) {
        setResult({ kind: "email", address: email });
      } else {
        onClose();
      }
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not save user");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/50 p-4" onClick={onClose}>
      <div
        onClick={(e) => e.stopPropagation()}
        className="w-[var(--width-dialog)] space-y-5 rounded-lg border border-surface-700 bg-surface-850 p-6"
      >
        {result ? (
          <>
            <h2 className="font-mono text-base font-medium text-text-bright">User created</h2>
            {result.kind === "password" ? (
              <div className="space-y-2">
                <p className="text-sm text-text-secondary">
                  Temporary password (shown once; the user must change it on first login):
                </p>
                <code className="block rounded-md border border-surface-700 bg-surface-950 px-3 py-2 text-sm text-text-bright">
                  {result.value}
                </code>
              </div>
            ) : (
              <p className="text-sm text-text-secondary">
                A setup email was sent to <span className="text-text-primary">{result.address}</span> with a link to set
                a password.
              </p>
            )}
            <div className="flex justify-end">
              <Button type="button" variant="primary" onClick={onClose}>
                Done
              </Button>
            </div>
          </>
        ) : (
          <form onSubmit={submit} className="space-y-5">
            <h2 className="font-mono text-base font-medium text-text-bright">{editing ? "Edit user" : "New user"}</h2>
            <Field label="Username">
              <Input value={username} onChange={(e) => setUsername(e.target.value)} autoFocus />
            </Field>
            <Field label="Email">
              <Input
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder={!editing && sendSetup ? "required for setup email" : "optional"}
              />
            </Field>

            {canManageRoles && (
              <Field label="Role">
                <Select
                  value={roleId ?? ""}
                  onChange={(e) => setRoleId(e.target.value ? Number(e.target.value) : null)}
                >
                  {roles.map((r) => (
                    <option key={r.id} value={r.id}>
                      {r.name}
                    </option>
                  ))}
                </Select>
              </Field>
            )}

            {!editing && (
              <label className="flex items-center gap-2 text-sm text-text-primary">
                <input
                  type="checkbox"
                  checked={sendSetup}
                  onChange={(e) => setSendSetup(e.target.checked)}
                  className="h-4 w-4 accent-brand-500"
                />
                Send setup email (user sets their own password)
              </label>
            )}

            {!(sendSetup && !editing) && (
              <Field label={editing ? "New password (leave blank to keep)" : "Password (leave blank to generate one)"}>
                <Input
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  autoComplete="new-password"
                />
              </Field>
            )}

            {error && <ErrorText>{error}</ErrorText>}
            <div className="flex justify-end gap-2">
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" disabled={busy}>
                {busy ? "Saving..." : "Save"}
              </Button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}
