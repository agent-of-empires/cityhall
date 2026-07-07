import { useState } from "react";
import { api, ApiError } from "../lib/api";
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
    <div className="flex h-full items-center justify-center p-4">
      <form
        onSubmit={submit}
        className="w-[var(--width-dialog)] space-y-5 rounded-lg border border-surface-700 bg-surface-850 p-6"
      >
        <div className="space-y-1">
          <h1 className="font-mono text-lg font-medium text-text-bright">Change password</h1>
          {forced && <p className="text-sm text-status-waiting">You must set a new password before continuing.</p>}
        </div>
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
        <Button type="submit" variant="primary" className="w-full" disabled={busy}>
          {busy ? "Saving..." : "Update password"}
        </Button>
      </form>
    </div>
  );
}
