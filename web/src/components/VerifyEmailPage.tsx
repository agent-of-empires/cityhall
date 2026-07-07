import { useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { ErrorText } from "./ui";

type State = "verifying" | "done" | "error";

export function VerifyEmailPage() {
  const [params] = useSearchParams();
  const token = params.get("token") ?? "";
  const [state, setState] = useState<State>(token ? "verifying" : "error");
  const [error, setError] = useState<string | null>(token ? null : "This link is missing its token.");
  const ran = useRef(false);

  useEffect(() => {
    if (!token || ran.current) return;
    ran.current = true;
    api
      .verifyEmail(token)
      .then(() => setState("done"))
      .catch((err) => {
        setError(err instanceof ApiError ? err.message : "could not verify email");
        setState("error");
      });
  }, [token]);

  return (
    <div className="flex h-full items-center justify-center p-4">
      <div className="w-[var(--width-dialog)] space-y-5 rounded-lg border border-surface-700 bg-surface-850 p-6">
        <h1 className="font-mono text-lg font-medium text-text-bright">Email verification</h1>
        {state === "verifying" && <p className="text-sm text-text-muted">Verifying...</p>}
        {state === "done" && (
          <>
            <p className="text-sm text-text-secondary">Your email is verified. You can now sign in.</p>
            <Link to="/login" className="text-sm text-brand-500 hover:text-brand-400">
              Go to sign in
            </Link>
          </>
        )}
        {state === "error" && (
          <>
            <ErrorText>{error}</ErrorText>
            <Link to="/login" className="text-sm text-brand-500 hover:text-brand-400">
              Back to sign in
            </Link>
          </>
        )}
      </div>
    </div>
  );
}
