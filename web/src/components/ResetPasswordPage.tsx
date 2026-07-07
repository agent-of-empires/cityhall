import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { Button, ErrorText, Field, Input } from "./ui";

export function ResetPasswordPage() {
  const [params] = useSearchParams();
  const token = params.get("token") ?? "";

  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (next !== confirm) {
      setError("passwords do not match");
      return;
    }
    setBusy(true);
    try {
      await api.resetPassword(token, next);
      setDone(true);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not reset password");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full items-center justify-center p-4">
      <div className="w-[var(--width-dialog)] space-y-5 rounded-lg border border-surface-700 bg-surface-850 p-6">
        <h1 className="font-mono text-lg font-medium text-text-bright">Set a new password</h1>
        {done ? (
          <>
            <p className="text-sm text-text-secondary">Your password has been set. You can now sign in.</p>
            <Link to="/login" className="text-sm text-brand-500 hover:text-brand-400">
              Go to sign in
            </Link>
          </>
        ) : !token ? (
          <>
            <ErrorText>This reset link is missing its token. Request a new one.</ErrorText>
            <Link to="/forgot-password" className="text-sm text-brand-500 hover:text-brand-400">
              Request a reset link
            </Link>
          </>
        ) : (
          <form onSubmit={submit} className="space-y-5">
            <Field label="New password">
              <Input
                type="password"
                value={next}
                onChange={(e) => setNext(e.target.value)}
                autoFocus
                autoComplete="new-password"
              />
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
              {busy ? "Saving..." : "Set password"}
            </Button>
          </form>
        )}
      </div>
    </div>
  );
}
