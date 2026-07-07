import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { Button, ErrorText, Field, Input } from "./ui";

export function LoginPage({ onAuthed }: { onAuthed: () => Promise<void> }) {
  const [params] = useSearchParams();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(params.get("error"));
  const [busy, setBusy] = useState(false);
  const [ssoEnabled, setSsoEnabled] = useState(false);

  useEffect(() => {
    api
      .providers()
      .then((p) => setSsoEnabled(p.oidc))
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
    <div className="flex h-full items-center justify-center p-4">
      <form
        onSubmit={submit}
        className="w-[var(--width-dialog)] space-y-5 rounded-lg border border-surface-700 bg-surface-850 p-6"
      >
        <div className="space-y-1">
          <h1 className="font-mono text-lg font-medium text-text-bright">CityHall</h1>
          <p className="text-sm text-text-muted">Sign in to continue</p>
        </div>
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
        <Button type="submit" variant="primary" className="w-full" disabled={busy}>
          {busy ? "Signing in..." : "Sign in"}
        </Button>
        {ssoEnabled && (
          <>
            <div className="flex items-center gap-3 text-xs text-text-muted">
              <span className="h-px flex-1 bg-surface-700" />
              or
              <span className="h-px flex-1 bg-surface-700" />
            </div>
            <Button
              type="button"
              variant="default"
              className="w-full"
              onClick={() => {
                window.location.href = "/api/auth/oidc/login";
              }}
            >
              Sign in with SSO
            </Button>
          </>
        )}
        <Link to="/forgot-password" className="block text-center text-sm text-text-muted hover:text-text-primary">
          Forgot password?
        </Link>
      </form>
    </div>
  );
}
