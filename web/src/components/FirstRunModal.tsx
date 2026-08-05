import clsx from "clsx";
import { useState } from "react";
import { Link } from "react-router-dom";
import { api } from "../lib/api";
import { Button, Modal, SectionLabel } from "./ui";

const STEP_COUNT = 3;

/// A member's guided tour of their workspace, shown once until dismissed (#61
/// design pass). Both "skip" and the final "Open workspace" persist the
/// dismissal server side, so the tour never nags a member who has already seen
/// it or chosen to move on.
export function FirstRunModal({ proxyOrigin, onFinish }: { proxyOrigin: string | null; onFinish: () => void }) {
  const [step, setStep] = useState(0);
  const [busy, setBusy] = useState(false);

  async function finish(openWorkspace: boolean) {
    setBusy(true);
    try {
      await api.dismissOnboarding();
    } catch {
      // A failed dismiss must not trap a member behind the modal; it just
      // means the tour may reappear next visit.
    } finally {
      setBusy(false);
    }
    if (openWorkspace && proxyOrigin) {
      window.open(`${proxyOrigin}/?cityhall_ws_exit=1`, "_blank", "noopener");
    }
    onFinish();
  }

  function next() {
    if (step >= STEP_COUNT - 1) void finish(true);
    else setStep(step + 1);
  }

  return (
    <Modal
      title={`Getting started · step ${step + 1} of ${STEP_COUNT}`}
      footer={
        <div className="flex w-full items-center justify-between gap-3">
          <Button variant="ghost" disabled={busy} onClick={() => void finish(false)}>
            {step === 0 ? "Skip" : "I'll do this later"}
          </Button>
          <div className="flex gap-2">
            <Button disabled={step === 0 || busy} onClick={() => setStep(step - 1)}>
              Back
            </Button>
            <Button variant="primary" disabled={busy} onClick={next} className="min-w-[104px]">
              {step === STEP_COUNT - 1 ? "Open workspace" : "Next"}
            </Button>
          </div>
        </div>
      }
    >
      <div className="space-y-4">
        <div className="flex gap-1.5">
          {Array.from({ length: STEP_COUNT }, (_, i) => (
            <span key={i} className={clsx("h-[3px] flex-1 rounded-full", i <= step ? "bg-accent" : "bg-border-soft")} />
          ))}
        </div>
        {step === 0 && <HowItWorksStep />}
        {step === 1 && <ConnectAccountsStep />}
        {step === 2 && <TerminalLoginStep />}
      </div>
    </Modal>
  );
}

function StepHeading({ title, subcopy }: { title: string; subcopy: string }) {
  return (
    <div>
      <h2 className="text-xl font-semibold tracking-[-0.02em] text-text-bright">{title}</h2>
      <p className="mt-1.5 text-[13.5px] leading-relaxed text-text-dim">{subcopy}</p>
    </div>
  );
}

function InfoTile({ eyebrow, body }: { eyebrow: string; body: string }) {
  return (
    <div className="rounded-md border border-border-soft p-3.5">
      <SectionLabel className="text-accent">{eyebrow}</SectionLabel>
      <p className="mt-1.5 text-[12.5px] leading-relaxed text-text-dim">{body}</p>
    </div>
  );
}

function HowItWorksStep() {
  return (
    <div className="space-y-3.5">
      <StepHeading
        title="How your workspace works"
        subcopy="One long-lived aoe instance, yours alone, with its own persistent volume."
      />
      <div className="grid grid-cols-3 gap-2.5">
        <InfoTile eyebrow="YOU OPEN IT" body="Starts on the first request. No start button." />
        <InfoTile eyebrow="IDLE TIMEOUT" body="Stops. Everything on the volume is kept." />
        <InfoTile eyebrow="YOU RETURN" body="Resumes exactly where you left it." />
      </div>
      <p className="text-[12.5px] leading-relaxed text-text-hint">
        You get the message composer and the structured view. Terminal and diff panes, project management and advanced
        settings are managed by your admin.
      </p>
    </div>
  );
}

function AccountLinkCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="flex flex-col gap-2 rounded-md border border-border-soft p-3.5">
      <span className="text-[13.5px] font-medium text-text-bright">{title}</span>
      <p className="text-[12.5px] leading-relaxed text-text-dim">{body}</p>
      <Link to="/account" className="font-mono text-[12.5px] text-link hover:text-accent-bright">
        Set up in Account →
      </Link>
    </div>
  );
}

function ConnectAccountsStep() {
  return (
    <div className="space-y-3">
      <StepHeading
        title="Connect your accounts"
        subcopy="Both are optional. Set them up on the Account page, then come back here whenever you like."
      />
      <AccountLinkCard
        title="Git credential"
        body="A personal access token for https:// clones, or an SSH key for git@ remotes. Commits made in your workspace are attributed to you."
      />
      <AccountLinkCard
        title="Agent credential"
        body="An API key or subscription token for the coding agent you use, so it can authenticate inside your workspace."
      />
    </div>
  );
}

function TerminalLoginStep() {
  return (
    <div className="space-y-3">
      <StepHeading
        title="Everything else signs in from a terminal"
        subcopy="Open a terminal session in your workspace and run the one you use. The login is written to the agent's own config directory, which lives on your volume, so it survives a restart."
      />
      <pre className="overflow-x-auto rounded-md border border-border-soft bg-code-bg p-3.5 font-mono text-[12.5px] leading-[1.9] text-text">
        <span className="text-text-faint">$</span> codex login{" "}
        <span className="text-text-hint"># ChatGPT subscription</span>
        {"\n"}
        <span className="text-text-faint">$</span> gemini <span className="text-text-hint"># prompts on first run</span>
        {"\n"}
        <span className="text-text-faint">$</span> opencode auth login{" "}
        <span className="text-text-hint"># per provider</span>
        {"\n"}
        <span className="text-text-faint">$</span> claude setup-token{" "}
        <span className="text-text-hint"># Pro/Max subscription</span>
      </pre>
      <p className="text-[12.5px] leading-relaxed text-text-hint">
        If an agent is missing entirely, aoe prints the exact{" "}
        <code className="font-mono text-text">npm install -g</code> command when you pick it.
      </p>
    </div>
  );
}
