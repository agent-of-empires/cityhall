import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { AuthCard } from "./AuthCard";
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
    <AuthCard>
      <h1 className="text-[26px] font-semibold tracking-[-0.02em] text-text-bright">Set a new password</h1>
      {done ? (
        <>
          <p className="text-sm text-text-dim">Your password has been set. You can now sign in.</p>
          <Link to="/login" className="text-sm text-accent hover:text-accent-bright">
            Go to sign in
          </Link>
        </>
      ) : !token ? (
        <>
          <ErrorText>This reset link is missing its token. Request a new one.</ErrorText>
          <Link to="/forgot-password" className="text-sm text-accent hover:text-accent-bright">
            Request a reset link
          </Link>
        </>
      ) : (
        <form onSubmit={submit} className="flex flex-col gap-3">
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
          <Button type="submit" variant="primary" className="mt-1 w-full" disabled={busy}>
            {busy ? "Saving..." : "Set password"}
          </Button>
        </form>
      )}
    </AuthCard>
  );
}
