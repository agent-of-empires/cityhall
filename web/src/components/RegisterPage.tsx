import { useState } from "react";
import { Link } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { AuthCard } from "./AuthCard";
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
    <AuthCard>
      <h1 className="text-[26px] font-semibold tracking-[-0.02em] text-text-bright">Create an account</h1>
      {done ? (
        <>
          <p className="text-sm text-text-dim">
            Check your inbox for a verification link. You can sign in once your email is confirmed.
          </p>
          <Link to="/login" className="text-sm text-accent hover:text-accent-bright">
            Back to sign in
          </Link>
        </>
      ) : (
        <form onSubmit={submit} className="flex flex-col gap-3">
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
          <Button type="submit" variant="primary" className="mt-1 w-full" disabled={busy}>
            {busy ? "Creating..." : "Create account"}
          </Button>
          <Link to="/login" className="block text-center text-sm text-text-dim hover:text-text-bright">
            Back to sign in
          </Link>
        </form>
      )}
    </AuthCard>
  );
}
