import { useState } from "react";
import { Link } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { AuthCard } from "./AuthCard";
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
    <AuthCard>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-[26px] font-semibold tracking-[-0.02em] text-text-bright">Forgot password</h1>
        {!sent && <p className="text-sm text-text-dim">Enter your email and we'll send a reset link.</p>}
      </div>
      {sent ? (
        <>
          <p className="text-sm text-text-dim">
            If an account exists for that email, a password reset link has been sent. Check your inbox.
          </p>
          <Link to="/login" className="text-sm text-accent hover:text-accent-bright">
            Back to sign in
          </Link>
        </>
      ) : (
        <form onSubmit={submit} className="flex flex-col gap-3">
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
          <Button type="submit" variant="primary" className="mt-1 w-full" disabled={busy}>
            {busy ? "Sending..." : "Send reset link"}
          </Button>
          <Link to="/login" className="block text-center text-sm text-text-dim hover:text-text-bright">
            Back to sign in
          </Link>
        </form>
      )}
    </AuthCard>
  );
}
