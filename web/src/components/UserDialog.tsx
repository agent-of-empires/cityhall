import { useEffect, useState } from "react";
import { api, ApiError, can, type Me, type Role, type User } from "../lib/api";
import { Button, Checkbox, ErrorText, Field, Input, Modal, Select } from "./ui";

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

  if (result) {
    return (
      <Modal
        title="User created"
        onClose={onClose}
        footer={
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        }
      >
        {result.kind === "password" ? (
          <div className="space-y-2">
            <p className="text-sm text-text-dim">
              Temporary password (shown once; the user must change it on first login):
            </p>
            <code className="block rounded-md border border-border-soft bg-canvas px-3 py-2 text-sm text-text-bright">
              {result.value}
            </code>
          </div>
        ) : (
          <p className="text-sm text-text-dim">
            A setup email was sent to <span className="text-text">{result.address}</span> with a link to set a password.
          </p>
        )}
      </Modal>
    );
  }

  return (
    <Modal
      title={editing ? "Edit user" : "New user"}
      onClose={onClose}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" form="user-dialog-form" variant="primary" disabled={busy}>
            {busy ? "Saving..." : "Save"}
          </Button>
        </>
      }
    >
      <form id="user-dialog-form" onSubmit={submit} className="space-y-5">
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
            <Select value={roleId ?? ""} onChange={(e) => setRoleId(e.target.value ? Number(e.target.value) : null)}>
              {roles.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name}
                </option>
              ))}
            </Select>
          </Field>
        )}

        {!editing && (
          <Checkbox
            checked={sendSetup}
            onChange={setSendSetup}
            label="Send setup email (user sets their own password)"
          />
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
      </form>
    </Modal>
  );
}
