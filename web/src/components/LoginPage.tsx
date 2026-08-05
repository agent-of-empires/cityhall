import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { AuthCard } from "./AuthCard";
import { Button, ErrorText, Field, Input } from "./ui";

export function LoginPage({ onAuthed }: { onAuthed: () => Promise<void> }) {
  const [params] = useSearchParams();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(params.get("error"));
  const [busy, setBusy] = useState(false);
  const [ssoEnabled, setSsoEnabled] = useState(false);
  const [signupEnabled, setSignupEnabled] = useState(false);

  useEffect(() => {
    api
      .providers()
      .then((p) => {
        setSsoEnabled(p.oidc);
        setSignupEnabled(p.signup);
      })
      .catch(() => {});
  }, []);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      await api.login(username, password);
      await onAuthed();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "login failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthCard>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-[26px] font-semibold tracking-[-0.02em] text-text-bright">Sign in</h1>
        <p className="text-sm text-text-dim">The control plane for your team's aoe workspaces.</p>
      </div>
      <form onSubmit={submit} className="flex flex-col gap-3">
        <Field label="Username">
          <Input value={username} onChange={(e) => setUsername(e.target.value)} autoFocus autoComplete="username" />
        </Field>
        <Field label="Password">
          <Input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="current-password"
          />
        </Field>
        {error && <ErrorText>{error}</ErrorText>}
        <Button type="submit" variant="primary" className="mt-1 w-full" disabled={busy}>
          {busy ? "Signing in..." : "Sign in"}
        </Button>
        {ssoEnabled && (
          <>
            <div className="flex items-center gap-3">
              <span className="h-px flex-1 bg-border-soft" />
              <span className="font-mono text-[11px] text-text-faint">OR</span>
              <span className="h-px flex-1 bg-border-soft" />
            </div>
            <Button
              type="button"
              className="w-full"
              onClick={() => {
                window.location.href = "/api/auth/oidc/login";
              }}
            >
              Continue with SSO
            </Button>
          </>
        )}
      </form>
      <p className="text-[12.5px] leading-relaxed text-text-hint">
        First launch? The seeded <span className="font-mono text-text-dim">admin</span> password is printed once in the
        server log, and you will be asked to replace it.
      </p>
      <div className="flex flex-col items-center gap-2 text-sm text-text-dim">
        <Link to="/forgot-password" className="hover:text-text-bright">
          Forgot password?
        </Link>
        {signupEnabled && (
          <Link to="/register" className="hover:text-text-bright">
            Create an account
          </Link>
        )}
      </div>
    </AuthCard>
  );
}
