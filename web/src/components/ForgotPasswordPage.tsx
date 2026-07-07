import { useState } from "react";
import { Link } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { Button, ErrorText, Field, Input } from "./ui";

export function ForgotPasswordPage() {
  const [email, setEmail] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [sent, setSent] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      await api.forgotPassword(email);
      setSent(true);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not submit request");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full items-center justify-center p-4">
      <div className="w-[var(--width-dialog)] space-y-5 rounded-lg border border-surface-700 bg-surface-850 p-6">
        <h1 className="font-mono text-lg font-medium text-text-bright">Forgot password</h1>
        {sent ? (
          <>
            <p className="text-sm text-text-secondary">
              If an account exists for that email, a password reset link has been sent. Check your inbox.
            </p>
            <Link to="/login" className="text-sm text-brand-500 hover:text-brand-400">
              Back to sign in
            </Link>
          </>
        ) : (
          <form onSubmit={submit} className="space-y-5">
            <p className="text-sm text-text-muted">Enter your email and we'll send a reset link.</p>
            <Field label="Email">
              <Input
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                autoFocus
                autoComplete="email"
              />
            </Field>
            {error && <ErrorText>{error}</ErrorText>}
            <Button type="submit" variant="primary" className="w-full" disabled={busy}>
              {busy ? "Sending..." : "Send reset link"}
            </Button>
            <Link to="/login" className="block text-center text-sm text-text-muted hover:text-text-primary">
              Back to sign in
            </Link>
          </form>
        )}
      </div>
    </div>
  );
}
