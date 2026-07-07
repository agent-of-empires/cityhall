import { useState } from "react";
import { api, ApiError, type User } from "../lib/api";
import { Button, ErrorText, Field, Input } from "./ui";

export function UserDialog({
  user,
  onClose,
  onSaved,
}: {
  user: User | null; // null = create mode
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const editing = user !== null;
  const [username, setUsername] = useState(user?.username ?? "");
  const [email, setEmail] = useState(user?.email ?? "");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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
        });
      } else {
        await api.createUser(username, email || null, password);
      }
      await onSaved();
      onClose();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not save user");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/50 p-4" onClick={onClose}>
      <form
        onSubmit={submit}
        onClick={(e) => e.stopPropagation()}
        className="w-[var(--width-dialog)] space-y-5 rounded-lg border border-surface-700 bg-surface-850 p-6"
      >
        <h2 className="font-mono text-base font-medium text-text-bright">{editing ? "Edit user" : "New user"}</h2>
        <Field label="Username">
          <Input value={username} onChange={(e) => setUsername(e.target.value)} autoFocus />
        </Field>
        <Field label="Email">
          <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="optional" />
        </Field>
        <Field label={editing ? "New password (leave blank to keep)" : "Password"}>
          <Input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="new-password"
          />
        </Field>
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
    </div>
  );
}
