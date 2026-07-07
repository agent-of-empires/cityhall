import { useState } from "react";
import { Link } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { Button, ErrorText, Field, Input } from "./ui";

export function RegisterPage() {
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (password !== confirm) {
      setError("passwords do not match");
      return;
    }
    setBusy(true);
    try {
      await api.register(username, email, password);
      setDone(true);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not create account");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full items-center justify-center p-4">
      <div className="w-[var(--width-dialog)] space-y-5 rounded-lg border border-surface-700 bg-surface-850 p-6">
        <h1 className="font-mono text-lg font-medium text-text-bright">Create an account</h1>
        {done ? (
          <>
            <p className="text-sm text-text-secondary">
              Check your inbox for a verification link. You can sign in once your email is confirmed.
            </p>
            <Link to="/login" className="text-sm text-brand-500 hover:text-brand-400">
              Back to sign in
            </Link>
          </>
        ) : (
          <form onSubmit={submit} className="space-y-5">
            <Field label="Username">
              <Input value={username} onChange={(e) => setUsername(e.target.value)} autoFocus autoComplete="username" />
            </Field>
            <Field label="Email">
              <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" />
            </Field>
            <Field label="Password">
              <Input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="new-password"
              />
            </Field>
            <Field label="Confirm password">
              <Input
                type="password"
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                autoComplete="new-password"
              />
            </Field>
            {error && <ErrorText>{error}</ErrorText>}
            <Button type="submit" variant="primary" className="w-full" disabled={busy}>
              {busy ? "Creating..." : "Create account"}
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
