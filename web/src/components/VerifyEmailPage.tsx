import { useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { api, ApiError } from "../lib/api";
import { AuthCard } from "./AuthCard";
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
    <AuthCard>
      <h1 className="text-[26px] font-semibold tracking-[-0.02em] text-text-bright">Email verification</h1>
      {state === "verifying" && <p className="text-sm text-text-dim">Verifying...</p>}
      {state === "done" && (
        <>
          <p className="text-sm text-text-dim">Your email is verified. You can now sign in.</p>
          <Link to="/login" className="text-sm text-accent hover:text-accent-bright">
            Go to sign in
          </Link>
        </>
      )}
      {state === "error" && (
        <>
          <ErrorText>{error}</ErrorText>
          <Link to="/login" className="text-sm text-accent hover:text-accent-bright">
            Back to sign in
          </Link>
        </>
      )}
    </AuthCard>
  );
}
