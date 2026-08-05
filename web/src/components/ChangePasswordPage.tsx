import { useState } from "react";
import { api, ApiError } from "../lib/api";
import { AuthCard } from "./AuthCard";
import { Button, ErrorText, Field, Input } from "./ui";

export function ChangePasswordPage({ forced, onDone }: { forced: boolean; onDone: () => Promise<void> }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (next !== confirm) {
      setError("passwords do not match");
      return;
    }
    setBusy(true);
    try {
      await api.changePassword(current, next);
      await onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not change password");
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthCard>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-[26px] font-semibold tracking-[-0.02em] text-text-bright">Change password</h1>
        {forced && <p className="text-sm text-waiting">You must set a new password before continuing.</p>}
      </div>
      <form onSubmit={submit} className="flex flex-col gap-3">
        <Field label="Current password">
          <Input
            type="password"
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
            autoFocus
            autoComplete="current-password"
          />
        </Field>
        <Field label="New password">
          <Input type="password" value={next} onChange={(e) => setNext(e.target.value)} autoComplete="new-password" />
        </Field>
        <Field label="Confirm new password">
          <Input
            type="password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            autoComplete="new-password"
          />
        </Field>
        {error && <ErrorText>{error}</ErrorText>}
        <Button type="submit" variant="primary" className="mt-1 w-full" disabled={busy}>
          {busy ? "Saving..." : "Update password"}
        </Button>
      </form>
    </AuthCard>
  );
}
