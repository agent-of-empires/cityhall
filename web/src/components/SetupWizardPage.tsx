import clsx from "clsx";
import { Check } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  api,
  ApiError,
  type AvailableAgent,
  type Me,
  type OidcSettings,
  type Role,
  type SignupSettings,
  type SmtpSettings,
  type User,
  type WorkspaceSettings,
} from "../lib/api";
import { parseBundle, readProjects, stringifyBundle, writeProjects, type ProjectRow } from "../lib/bundleForm";
import { fetchSetupInputs } from "../lib/setupInputs";
import { SETUP_STEPS, setupSteps, type SetupInputs } from "../lib/setupProgress";
import { Button, Card, ErrorText, Field, Input, Select, SectionLabel, Toggle } from "./ui";

/** Tick or untick one agent, keeping the selection in the catalog's order (the
 *  server stores a canonical set, and a reordered-but-identical list would
 *  otherwise read as a change). Kept local rather than imported from
 *  WorkspaceSettings.tsx, which owns that file for its own settings page. */
function toggleAgentLocal(selected: string[], catalog: AvailableAgent[], name: string, on: boolean): string[] {
  const next = new Set(selected);
  if (on) next.add(name);
  else next.delete(name);
  return catalog.filter((a) => next.has(a.name)).map((a) => a.name);
}

/** The admin setup wizard (#59-ish): a full-screen, seven-step walkthrough of
 *  the settings a fresh CityHall needs before a team can use it. Every step
 *  reads and writes through the same endpoints as the Settings page; this is
 *  just a guided front door to them. Re-enterable: it always opens on the
 *  first step that is not yet configured or dismissed. */
export function SetupWizardPage({ me, onFinish }: { me: Me; onFinish: () => void }) {
  const navigate = useNavigate();

  const [phase, setPhase] = useState<"loading" | "ready">("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [stepIndex, setStepIndex] = useState(0);
  const [dismissed, setDismissed] = useState<string[]>([]);
  const [inputs, setInputs] = useState<SetupInputs | null>(null);

  const [stepError, setStepError] = useState<string | null>(null);
  const [stepSaving, setStepSaving] = useState(false);

  // Steps: aoe version / coding agents (one settings area, two steps)
  const [wsSettings, setWsSettings] = useState<WorkspaceSettings | null>(null);
  const [defaultVersion, setDefaultVersion] = useState("");
  const [agents, setAgents] = useState<string[]>([]);
  const [availableAgents, setAvailableAgents] = useState<AvailableAgent[]>([]);
  const [versions, setVersions] = useState<string[]>([]);
  const [latestVersion, setLatestVersion] = useState<string | null>(null);

  // Step: projects (workspace config bundle)
  const [configLoaded, setConfigLoaded] = useState(false);
  const [configBundle, setConfigBundle] = useState("");

  // Step: email (SMTP)
  const [smtp, setSmtp] = useState<SmtpSettings | null>(null);
  const [smtpHost, setSmtpHost] = useState("");
  const [smtpPort, setSmtpPort] = useState(587);
  const [smtpEncryption, setSmtpEncryption] = useState("starttls");
  const [smtpUsername, setSmtpUsername] = useState("");
  const [smtpPassword, setSmtpPassword] = useState("");
  const [smtpFromAddress, setSmtpFromAddress] = useState("");
  const [smtpFromName, setSmtpFromName] = useState("");
  const [smtpEnabled, setSmtpEnabled] = useState(false);
  const [testTo, setTestTo] = useState("");
  const [testSending, setTestSending] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; error: string | null } | null>(null);

  // Step: sign-in & SSO
  const [oidc, setOidc] = useState<OidcSettings | null>(null);
  const [ssoEnabled, setSsoEnabled] = useState(false);
  const [issuer, setIssuer] = useState("");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [scopes, setScopes] = useState("openid email profile");
  const [oidcAllowedDomains, setOidcAllowedDomains] = useState("");
  const [signup, setSignup] = useState<SignupSettings | null>(null);
  const [signupEnabled, setSignupEnabled] = useState(false);
  const [signupAllowedDomains, setSignupAllowedDomains] = useState("");
  const [signupDefaultRoleId, setSignupDefaultRoleId] = useState<number | null>(null);

  // Step: invite your team
  const [roles, setRoles] = useState<Role[]>([]);
  const [users, setUsers] = useState<User[]>([]);
  const [inviteUsername, setInviteUsername] = useState("");
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteRoleId, setInviteRoleId] = useState<number | null>(null);
  const [inviteNotes, setInviteNotes] = useState<{ username: string; note: string }[]>([]);

  const applySmtp = useCallback((s: SmtpSettings) => {
    setSmtp(s);
    setSmtpHost(s.host);
    setSmtpPort(s.port);
    setSmtpEncryption(s.encryption);
    setSmtpUsername(s.username ?? "");
    setSmtpFromAddress(s.from_address);
    setSmtpFromName(s.from_name ?? "");
    setSmtpEnabled(s.enabled);
    setSmtpPassword("");
  }, []);

  const applyOidc = useCallback((s: OidcSettings) => {
    setOidc(s);
    setSsoEnabled(s.enabled);
    setIssuer(s.issuer);
    setClientId(s.client_id);
    setScopes(s.scopes);
    setOidcAllowedDomains(s.allowed_domains);
    setClientSecret("");
  }, []);

  const applySignup = useCallback((s: SignupSettings) => {
    setSignup(s);
    setSignupEnabled(s.signup_enabled);
    setSignupAllowedDomains(s.signup_allowed_domains);
    setSignupDefaultRoleId(s.signup_default_role_id);
  }, []);

  const load = useCallback(async () => {
    setPhase("loading");
    setLoadError(null);
    try {
      const [
        state,
        inputsResult,
        ws,
        versionsResult,
        config,
        smtpResult,
        oidcResult,
        signupResult,
        rolesResult,
        usersResult,
      ] = await Promise.all([
        api.getSetupState(),
        fetchSetupInputs(me.must_change_password),
        api.getWorkspaceSettings().catch(() => null),
        api.listWorkspaceVersions().catch(() => null),
        api.getWorkspaceConfig().catch(() => null),
        api.getSmtpSettings().catch(() => null),
        api.getOidcSettings().catch(() => null),
        api.getSignupSettings().catch(() => null),
        api.listRoles().catch(() => [] as Role[]),
        api.listUsers().catch(() => [] as User[]),
      ]);

      setDismissed(state.dismissed_steps);

      if (ws) {
        setWsSettings(ws);
        setDefaultVersion(ws.default_version ?? "");
        setAgents(ws.agents);
        setAvailableAgents(ws.available_agents);
      }
      if (versionsResult) {
        setVersions(versionsResult.versions);
        setLatestVersion(versionsResult.latest);
      }
      if (config) {
        setConfigBundle(config.bundle);
        setConfigLoaded(true);
      }
      if (smtpResult) applySmtp(smtpResult);
      if (oidcResult) applyOidc(oidcResult);
      if (signupResult) applySignup(signupResult);

      setRoles(rolesResult);
      const memberRole = rolesResult.find((r) => r.name === "member");
      setInviteRoleId(memberRole?.id ?? rolesResult[0]?.id ?? null);
      setUsers(usersResult);
      setInputs(inputsResult);

      const steps = setupSteps(inputsResult, state.dismissed_steps);
      const firstUnfinished = steps.findIndex((s) => !s.done);
      setStepIndex(firstUnfinished === -1 ? steps.length - 1 : firstUnfinished);
    } catch (e) {
      setLoadError(e instanceof ApiError ? e.message : "could not load the setup wizard");
    } finally {
      setPhase("ready");
    }
  }, [me.must_change_password, applySmtp, applyOidc, applySignup]);

  useEffect(() => {
    void load();
  }, [load]);

  function goToStep(i: number) {
    setStepError(null);
    setStepIndex(Math.max(0, Math.min(SETUP_STEPS.length - 1, i)));
  }

  async function saveWorkspaceSettingsStep(): Promise<boolean> {
    if (!wsSettings) {
      setStepError("workspace settings failed to load; reload the page before continuing");
      return false;
    }
    try {
      const updated = await api.updateWorkspaceSettings({
        image_template: wsSettings.image_template,
        default_version: defaultVersion.trim() || null,
        idle_stop_minutes: wsSettings.idle_stop_minutes,
        telemetry_policy: wsSettings.telemetry_policy,
        restart_running: false,
        agents,
      });
      setWsSettings(updated);
      setDefaultVersion(updated.default_version ?? "");
      setAgents(updated.agents);
      setAvailableAgents(updated.available_agents);
      setInputs((prev) =>
        prev ? { ...prev, defaultVersion: updated.default_version, agentCount: updated.agents.length } : prev,
      );
      return true;
    } catch (e) {
      setStepError(e instanceof ApiError ? e.message : "could not save workspace settings");
      return false;
    }
  }

  async function saveVersionStep(): Promise<boolean> {
    if (!defaultVersion.trim()) {
      setStepError("choose a default aoe version; workspaces cannot start without one");
      return false;
    }
    return saveWorkspaceSettingsStep();
  }

  const parsedBundle = parseBundle(configBundle);
  const projectDoc = "doc" in parsedBundle ? parsedBundle.doc : null;
  const bundleParseError = "error" in parsedBundle ? parsedBundle.error : null;
  const projectRows = projectDoc ? readProjects(projectDoc) : [];

  function setProjectRows(rows: ProjectRow[]) {
    if (!projectDoc) return;
    setConfigBundle(stringifyBundle(writeProjects(projectDoc, rows)));
  }

  async function saveProjectsStep(): Promise<boolean> {
    if (!configLoaded || !projectDoc) {
      setStepError(
        bundleParseError
          ? `the saved workspace config is not valid TOML (${bundleParseError}); fix it from Settings first`
          : "the workspace config failed to load; reload the page before continuing",
      );
      return false;
    }
    try {
      const updated = await api.updateWorkspaceConfig(configBundle);
      setConfigBundle(updated.bundle);
      setInputs((prev) => (prev ? { ...prev, projectCount: updated.summary.projects.length } : prev));
      return true;
    } catch (e) {
      setStepError(e instanceof ApiError ? e.message : "could not save the workspace config");
      return false;
    }
  }

  async function saveEmailStep(): Promise<boolean> {
    if (!smtp) {
      setStepError("email settings failed to load; reload the page before continuing");
      return false;
    }
    if (smtp.env_managed) return true;
    try {
      const updated = await api.updateSmtpSettings({
        host: smtpHost,
        port: smtpPort,
        encryption: smtpEncryption,
        username: smtpUsername || null,
        ...(smtpPassword ? { password: smtpPassword } : {}),
        from_address: smtpFromAddress,
        from_name: smtpFromName || null,
        enabled: smtpEnabled,
      });
      applySmtp(updated);
      setInputs((prev) => (prev ? { ...prev, emailReady: updated.enabled || updated.env_managed } : prev));
      return true;
    } catch (e) {
      setStepError(e instanceof ApiError ? e.message : "could not save email settings");
      return false;
    }
  }

  async function sendTestEmail() {
    setTestSending(true);
    setTestResult(null);
    try {
      setTestResult(await api.testSmtp(testTo));
    } catch (e) {
      setTestResult({ ok: false, error: e instanceof ApiError ? e.message : "could not send the test email" });
    } finally {
      setTestSending(false);
    }
  }

  async function saveSsoStep(): Promise<boolean> {
    if (!oidc || !signup) {
      setStepError("sign-in settings failed to load; reload the page before continuing");
      return false;
    }
    try {
      let effectiveOidcEnabled = oidc.enabled;
      if (!oidc.env_managed) {
        const updatedOidc = await api.updateOidcSettings({
          enabled: ssoEnabled,
          issuer,
          client_id: clientId,
          client_secret: clientSecret || undefined,
          scopes,
          allowed_domains: oidcAllowedDomains,
        });
        applyOidc(updatedOidc);
        effectiveOidcEnabled = updatedOidc.enabled;
      }
      const updatedSignup = await api.updateSignupSettings({
        signup_enabled: signupEnabled,
        signup_allowed_domains: signupAllowedDomains,
        signup_default_role_id: signupDefaultRoleId,
      });
      applySignup(updatedSignup);
      setInputs((prev) => (prev ? { ...prev, ssoReady: effectiveOidcEnabled || updatedSignup.signup_enabled } : prev));
      return true;
    } catch (e) {
      setStepError(e instanceof ApiError ? e.message : "could not save sign-in settings");
      return false;
    }
  }

  async function addInvite() {
    setStepError(null);
    if (!inviteUsername.trim()) {
      setStepError("a username is required");
      return;
    }
    const smtpReady = Boolean(smtp && (smtp.enabled || smtp.env_managed));
    setStepSaving(true);
    try {
      const res = await api.createUser({
        username: inviteUsername.trim(),
        email: inviteEmail.trim() || null,
        sendSetupEmail: smtpReady,
        roleId: inviteRoleId ?? undefined,
      });
      const note = res.generated_password ? `temporary password: ${res.generated_password}` : "setup email sent";
      setInviteNotes((prev) => [...prev, { username: res.username, note }]);
      setUsers((prev) => [...prev, res]);
      setInputs((prev) => (prev ? { ...prev, userCount: prev.userCount + 1 } : prev));
      setInviteUsername("");
      setInviteEmail("");
    } catch (e) {
      setStepError(e instanceof ApiError ? e.message : "could not create the user");
    } finally {
      setStepSaving(false);
    }
  }

  async function saveCurrentStep(): Promise<boolean> {
    switch (SETUP_STEPS[stepIndex].key) {
      // Nothing to save: the password is always already set by the time the
      // wizard can be reached (App sends `must_change_password` to
      // /change-password first), so this step only reports that.
      case "password":
        return true;
      case "version":
        return saveVersionStep();
      case "agents":
        return saveWorkspaceSettingsStep();
      case "projects":
        return saveProjectsStep();
      case "email":
        return saveEmailStep();
      case "sso":
        return saveSsoStep();
      case "invites":
        // Invites are created immediately by the Add button; there is nothing
        // left to save when Next or Finish is pressed.
        return true;
    }
  }

  const isLast = stepIndex === SETUP_STEPS.length - 1;

  /** Marking setup finished with no default version would leave a deployment
   *  where no workspace can start and no checklist entry pushing anyone to fix
   *  it. The step rail can jump past a step, so every path that finishes has to
   *  re-check rather than trusting the order steps were visited in. */
  async function finishWizard(dismissedNow: string[] = dismissed) {
    const outstanding = inputs ? setupSteps(inputs, dismissedNow).find((s) => s.required && !s.done) : undefined;
    if (outstanding) {
      setStepIndex(SETUP_STEPS.findIndex((s) => s.key === outstanding.key));
      setStepError(`${outstanding.label} still needs to be set before setup can be finished`);
      return;
    }
    try {
      await api.updateSetupState({ dismissed_steps: dismissedNow, wizard_finished: true });
      setDismissed(dismissedNow);
      onFinish();
      navigate("/dashboard");
    } catch (e) {
      setStepError(e instanceof ApiError ? e.message : "could not finish setup");
    }
  }

  async function handleNext() {
    setStepError(null);
    setStepSaving(true);
    try {
      const ok = await saveCurrentStep();
      if (!ok) return;
      if (isLast) await finishWizard();
      else setStepIndex((i) => Math.min(SETUP_STEPS.length - 1, i + 1));
    } finally {
      setStepSaving(false);
    }
  }

  async function handleSkip() {
    setStepError(null);
    const key = SETUP_STEPS[stepIndex].key;
    const nextDismissed = dismissed.includes(key) ? dismissed : [...dismissed, key];
    setStepSaving(true);
    try {
      // Skipping the last step ends the wizard, so it goes through the same
      // required-step guard as the Finish button.
      if (isLast) {
        await finishWizard(nextDismissed);
        return;
      }
      await api.updateSetupState({ dismissed_steps: nextDismissed, wizard_finished: false });
      setDismissed(nextDismissed);
      setStepIndex((i) => Math.min(SETUP_STEPS.length - 1, i + 1));
    } catch (e) {
      setStepError(e instanceof ApiError ? e.message : "could not update the setup state");
    } finally {
      setStepSaving(false);
    }
  }

  if (phase === "loading") {
    return <div className="flex h-screen items-center justify-center bg-canvas text-text-dim">Loading setup...</div>;
  }

  if (loadError || !inputs) {
    return (
      <div className="flex h-screen flex-col items-center justify-center gap-3 bg-canvas">
        <ErrorText>{loadError ?? "could not load the setup wizard"}</ErrorText>
        <Button onClick={() => void load()}>Retry</Button>
      </div>
    );
  }

  const steps = setupSteps(inputs, dismissed);
  const currentStepDef = SETUP_STEPS[stepIndex];
  const smtpReady = Boolean(smtp && (smtp.enabled || smtp.env_managed));
  const callbackUrl = oidc ? `${window.location.origin}${oidc.callback_path}` : "";

  return (
    <div className="flex h-screen bg-canvas text-text">
      <aside className="flex w-[288px] shrink-0 flex-col border-r border-border-soft bg-surface py-[22px]">
        <div className="flex items-center gap-2.5 px-[22px] pb-5">
          <img src="/logo.svg" alt="" className="h-[22px] w-[22px]" />
          <span className="font-mono text-[13.5px] font-semibold text-text-bright">CityHall setup</span>
        </div>
        <div className="flex flex-col gap-px px-3">
          {steps.map((step, i) => {
            const current = i === stepIndex;
            return (
              <button
                key={step.key}
                type="button"
                onClick={() => goToStep(i)}
                className={clsx(
                  "flex items-center gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors hover:bg-surface-hover",
                  current && "bg-surface-active",
                )}
              >
                <span
                  className={clsx(
                    "flex h-[19px] w-[19px] shrink-0 items-center justify-center rounded-[4px] border font-mono text-[10.5px]",
                    current
                      ? "border-accent bg-accent-soft text-accent-bright"
                      : step.done
                        ? "border-transparent text-running"
                        : "border-border text-text-hint",
                  )}
                >
                  {step.done ? <Check size={11} strokeWidth={2.5} /> : i + 1}
                </span>
                <span className={clsx("text-[13px]", current ? "font-medium text-text-bright" : "text-text-dim")}>
                  {step.label}
                </span>
              </button>
            );
          })}
        </div>
        <div className="flex-1" />
        <p className="px-[22px] text-[12.5px] leading-relaxed text-text-hint">
          Leave whenever you like. Anything unfinished waits for you as a checklist on the dashboard.
        </p>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex-1 overflow-y-auto">
          <div className="max-w-[640px] px-16 pt-14 pb-10">
            <div className="mb-3.5 font-mono text-[11px] tracking-[0.13em] text-accent uppercase">
              Step {stepIndex + 1} of {SETUP_STEPS.length}
            </div>

            {currentStepDef.key === "password" && <PasswordStep />}

            {currentStepDef.key === "version" && (
              <VersionStep
                defaultVersion={defaultVersion}
                setDefaultVersion={setDefaultVersion}
                versions={versions}
                latestVersion={latestVersion}
                imageTemplate={wsSettings?.image_template ?? ""}
              />
            )}

            {currentStepDef.key === "agents" && (
              <AgentsStep
                agents={agents}
                availableAgents={availableAgents}
                onToggle={(name, on) => setAgents((prev) => toggleAgentLocal(prev, availableAgents, name, on))}
              />
            )}

            {currentStepDef.key === "projects" &&
              (bundleParseError ? (
                <StepShell
                  title="What should be cloned into every workspace?"
                  subcopy="Projects are addressed by git remote, not path, so an admin's local checkout means nothing inside a container."
                >
                  <ErrorText>
                    The saved workspace config is not valid TOML ({bundleParseError}). Fix it from Settings, then come
                    back here.
                  </ErrorText>
                </StepShell>
              ) : (
                <ProjectsStep
                  rows={projectRows}
                  onChangeRow={(i, patch) =>
                    setProjectRows(projectRows.map((row, j) => (j === i ? { ...row, ...patch } : row)))
                  }
                  onAdd={() =>
                    setProjectRows([...projectRows, { name: "", remote: "", default_base_branch: "", extra: {} }])
                  }
                  onRemove={(i) => setProjectRows(projectRows.filter((_, j) => j !== i))}
                />
              ))}

            {currentStepDef.key === "email" && (
              <EmailStep
                envManaged={smtp?.env_managed ?? false}
                host={smtpHost}
                setHost={setSmtpHost}
                port={smtpPort}
                setPort={setSmtpPort}
                encryption={smtpEncryption}
                setEncryption={setSmtpEncryption}
                username={smtpUsername}
                setUsername={setSmtpUsername}
                password={smtpPassword}
                setPassword={setSmtpPassword}
                passwordSet={smtp?.password_set ?? false}
                fromAddress={smtpFromAddress}
                setFromAddress={setSmtpFromAddress}
                fromName={smtpFromName}
                setFromName={setSmtpFromName}
                enabled={smtpEnabled}
                setEnabled={setSmtpEnabled}
                testTo={testTo}
                setTestTo={setTestTo}
                testSending={testSending}
                testResult={testResult}
                onSendTest={() => void sendTestEmail()}
              />
            )}

            {currentStepDef.key === "sso" && (
              <SsoStep
                oidcEnvManaged={oidc?.env_managed ?? false}
                ssoEnabled={ssoEnabled}
                setSsoEnabled={setSsoEnabled}
                issuer={issuer}
                setIssuer={setIssuer}
                clientId={clientId}
                setClientId={setClientId}
                clientSecret={clientSecret}
                setClientSecret={setClientSecret}
                clientSecretSet={oidc?.client_secret_set ?? false}
                scopes={scopes}
                setScopes={setScopes}
                oidcAllowedDomains={oidcAllowedDomains}
                setOidcAllowedDomains={setOidcAllowedDomains}
                callbackUrl={callbackUrl}
                signupEnabled={signupEnabled}
                setSignupEnabled={setSignupEnabled}
                signupAllowedDomains={signupAllowedDomains}
                setSignupAllowedDomains={setSignupAllowedDomains}
                signupDefaultRoleId={signupDefaultRoleId}
                setSignupDefaultRoleId={setSignupDefaultRoleId}
                roles={roles}
              />
            )}

            {currentStepDef.key === "invites" && (
              <InvitesStep
                smtpReady={smtpReady}
                username={inviteUsername}
                setUsername={setInviteUsername}
                email={inviteEmail}
                setEmail={setInviteEmail}
                roleId={inviteRoleId}
                setRoleId={setInviteRoleId}
                roles={roles}
                onAdd={() => void addInvite()}
                adding={stepSaving}
                invited={inviteNotes}
                totalUsers={users.length}
              />
            )}

            {stepError && (
              <div className="mt-4 max-w-[560px]">
                <ErrorText>{stepError}</ErrorText>
              </div>
            )}
          </div>
        </div>

        <div className="flex items-center justify-between border-t border-border-soft bg-surface px-16 py-3.5">
          <div>
            {!currentStepDef.required && (
              <Button variant="ghost" onClick={() => void handleSkip()} disabled={stepSaving}>
                {isLast ? "Skip for now" : "Skip this step"}
              </Button>
            )}
          </div>
          <div className="flex gap-2.5">
            <Button onClick={() => goToStep(stepIndex - 1)} disabled={stepIndex === 0 || stepSaving}>
              Back
            </Button>
            <Button variant="primary" onClick={() => void handleNext()} disabled={stepSaving} className="min-w-[104px]">
              {stepSaving ? "Saving..." : isLast ? "Finish setup" : "Next"}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

/** Shared h1 + subcopy header every step opens with. */
function StepShell({ title, subcopy, children }: { title: string; subcopy: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-col gap-[22px]">
      <div className="flex flex-col gap-2">
        <h1 className="text-[28px] font-semibold tracking-[-0.02em] text-text-bright">{title}</h1>
        <p className="max-w-[560px] text-[15px] leading-relaxed text-text-dim">{subcopy}</p>
      </div>
      {children}
    </div>
  );
}

/** Reached only after the seeded password has been replaced: the forced
 *  /change-password screen runs before the wizard, so this step reports rather
 *  than asks. */
function PasswordStep() {
  return (
    <StepShell
      title="Admin password"
      subcopy="You replaced the password CityHall seeded on first launch, which mattered because it was written to the server log in plain text. You can change it again any time from the sign-in screen's password reset."
    >
      <Card className="max-w-[400px] text-[13px] text-running">Your password has been changed.</Card>
    </StepShell>
  );
}

function VersionStep({
  defaultVersion,
  setDefaultVersion,
  versions,
  latestVersion,
  imageTemplate,
}: {
  defaultVersion: string;
  setDefaultVersion: (v: string) => void;
  versions: string[];
  latestVersion: string | null;
  imageTemplate: string;
}) {
  return (
    <StepShell
      title="Which aoe version do workspaces run?"
      subcopy={
        <>
          The image for a workspace is the template with{" "}
          <code className="rounded-[3px] bg-surface-elevated px-1 font-mono text-[13px] text-text">{"{version}"}</code>{" "}
          substituted. A missing image is pulled, or built locally from the reference Dockerfile; the first start of a
          new version takes a few minutes.
        </>
      }
    >
      <div className="grid max-w-[520px] grid-cols-2 gap-3.5">
        <Field
          label="Default version"
          help={versions.length === 0 ? "No release catalog available; type a version tag directly." : undefined}
        >
          <Input
            list="wizard-aoe-versions"
            value={defaultVersion}
            onChange={(e) => setDefaultVersion(e.target.value)}
            placeholder="v1.13.2"
          />
          <datalist id="wizard-aoe-versions">
            {versions.map((v) => (
              <option key={v} value={v}>
                {v === latestVersion ? `${v} (latest)` : v}
              </option>
            ))}
          </datalist>
        </Field>
        <Field label="Image template">
          <Input value={imageTemplate} readOnly disabled className="font-mono text-[13px]" />
        </Field>
      </div>
      <Card className="flex max-w-[520px] gap-2.5">
        <span className="font-mono text-[13px] text-branch">◆</span>
        <p className="text-[13px] leading-relaxed text-text-dim">
          Running an unreleased commit? Type the tag you built (e.g.{" "}
          <code className="font-mono text-text">main-20260804</code>) directly into the field above.
        </p>
      </Card>
    </StepShell>
  );
}

function AgentsStep({
  agents,
  availableAgents,
  onToggle,
}: {
  agents: string[];
  availableAgents: AvailableAgent[];
  onToggle: (name: string, on: boolean) => void;
}) {
  return (
    <StepShell
      title="Which agents should a workspace arrive with?"
      subcopy="Ticked agents install themselves the first time a workspace boots, so someone opens one that is ready to use. This is a default, not a restriction: a user can install and run anything else."
    >
      <div className="grid max-w-[520px] grid-cols-2 gap-2">
        {availableAgents.map((agent) => {
          const on = agents.includes(agent.name);
          return (
            <button
              key={agent.name}
              type="button"
              aria-pressed={on}
              onClick={() => onToggle(agent.name, !on)}
              className={clsx(
                "flex items-center gap-2.5 rounded-md border px-3.5 py-3 text-left text-[13.5px] font-medium transition-colors",
                on
                  ? "border-accent bg-accent-soft text-text-bright"
                  : "border-border-soft bg-surface-elevated text-text-dim hover:border-border",
              )}
            >
              <span
                className={clsx(
                  "flex h-4 w-4 shrink-0 items-center justify-center rounded-[4px] border-[1.5px] text-white",
                  on ? "border-accent bg-accent" : "border-border",
                )}
              >
                {on && <Check size={11} strokeWidth={3} />}
              </span>
              {agent.label}
            </button>
          );
        })}
        {availableAgents.length === 0 && (
          <p className="col-span-2 text-[13px] text-text-dim">No agents available from the server catalog.</p>
        )}
      </div>
      <Card className="flex max-w-[520px] gap-2.5">
        <span className="font-mono text-[13px] text-waiting">!</span>
        <p className="text-[13px] leading-relaxed text-text-dim">
          The install runs in the background at boot and takes a minute or two. A workspace opened immediately may still
          be missing one; it appears on its own.
        </p>
      </Card>
    </StepShell>
  );
}

function ProjectsStep({
  rows,
  onChangeRow,
  onAdd,
  onRemove,
}: {
  rows: ProjectRow[];
  onChangeRow: (i: number, patch: Partial<ProjectRow>) => void;
  onAdd: () => void;
  onRemove: (i: number) => void;
}) {
  return (
    <StepShell
      title="What should be cloned into every workspace?"
      subcopy="A locked-down workspace cannot add its own projects, so it starts with nothing to launch a session against. Projects are addressed by git remote, not path: an admin's local checkout means nothing inside a container."
    >
      <div className="flex max-w-[600px] flex-col gap-2.5">
        {rows.map((row, i) => (
          <div key={i} className="grid grid-cols-[1fr_1.6fr_0.8fr_auto] items-end gap-2">
            <Field label="Name">
              <Input
                value={row.name}
                onChange={(e) => onChangeRow(i, { name: e.target.value })}
                placeholder="my-repo"
              />
            </Field>
            <Field label="Git remote">
              <Input
                value={row.remote}
                onChange={(e) => onChangeRow(i, { remote: e.target.value })}
                placeholder="https://github.com/org/my-repo.git"
                className="font-mono text-[12px]"
              />
            </Field>
            <Field label="Branch">
              <Input
                value={row.default_base_branch}
                onChange={(e) => onChangeRow(i, { default_base_branch: e.target.value })}
                placeholder="detected"
              />
            </Field>
            <Button type="button" variant="ghost" onClick={() => onRemove(i)}>
              Remove
            </Button>
          </div>
        ))}
        {rows.length === 0 && <p className="text-[13px] text-text-dim">No projects yet. Workspaces start empty.</p>}
        <Button type="button" onClick={onAdd} className="self-start">
          + Add project
        </Button>
      </div>
      <Card className="flex max-w-[600px] gap-2.5">
        <span className="font-mono text-[13px] text-branch">◆</span>
        <p className="text-[13px] leading-relaxed text-text-dim">
          Already have a configured aoe install? Run <code className="font-mono text-text">aoe cityhall export</code>{" "}
          and paste the resulting TOML from Settings &rarr; Workspace config instead.
        </p>
      </Card>
    </StepShell>
  );
}

function EmailStep({
  envManaged,
  host,
  setHost,
  port,
  setPort,
  encryption,
  setEncryption,
  username,
  setUsername,
  password,
  setPassword,
  passwordSet,
  fromAddress,
  setFromAddress,
  fromName,
  setFromName,
  enabled,
  setEnabled,
  testTo,
  setTestTo,
  testSending,
  testResult,
  onSendTest,
}: {
  envManaged: boolean;
  host: string;
  setHost: (v: string) => void;
  port: number;
  setPort: (v: number) => void;
  encryption: string;
  setEncryption: (v: string) => void;
  username: string;
  setUsername: (v: string) => void;
  password: string;
  setPassword: (v: string) => void;
  passwordSet: boolean;
  fromAddress: string;
  setFromAddress: (v: string) => void;
  fromName: string;
  setFromName: (v: string) => void;
  enabled: boolean;
  setEnabled: (v: boolean) => void;
  testTo: string;
  setTestTo: (v: string) => void;
  testSending: boolean;
  testResult: { ok: boolean; error: string | null } | null;
  onSendTest: () => void;
}) {
  return (
    <StepShell
      title="Send email, so invites arrive"
      subcopy="Without SMTP you can still create accounts, but you hand out the passwords yourself."
    >
      {envManaged && (
        <Card className="max-w-[520px] text-[13px] text-text-dim">
          SMTP is configured through environment variables, so these fields are read-only.
        </Card>
      )}
      <div className="grid max-w-[520px] grid-cols-2 gap-3.5">
        <Field label="Host">
          <Input
            value={host}
            disabled={envManaged}
            onChange={(e) => setHost(e.target.value)}
            placeholder="smtp.example.com"
          />
        </Field>
        <Field label="Port">
          <Input
            type="number"
            value={port}
            disabled={envManaged}
            onChange={(e) => setPort(Number(e.target.value))}
            className="font-mono"
          />
        </Field>
        <Field label="Encryption">
          <Select value={encryption} disabled={envManaged} onChange={(e) => setEncryption(e.target.value)}>
            <option value="none">None</option>
            <option value="starttls">STARTTLS</option>
            <option value="tls">TLS</option>
          </Select>
        </Field>
        <Field label="Username">
          <Input
            value={username}
            disabled={envManaged}
            onChange={(e) => setUsername(e.target.value)}
            placeholder="optional"
          />
        </Field>
        <Field label="From address">
          <Input
            type="email"
            value={fromAddress}
            disabled={envManaged}
            onChange={(e) => setFromAddress(e.target.value)}
          />
        </Field>
        <Field label="From name">
          <Input
            value={fromName}
            disabled={envManaged}
            onChange={(e) => setFromName(e.target.value)}
            placeholder="CityHall"
          />
        </Field>
        <Field label="Password">
          <Input
            type="password"
            value={password}
            disabled={envManaged}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={passwordSet ? "•••••••• (unchanged)" : "optional"}
            autoComplete="new-password"
          />
        </Field>
      </div>
      <div className="flex max-w-[520px] items-center gap-2.5">
        <Toggle checked={enabled} onChange={setEnabled} label="Enable SMTP" disabled={envManaged} />
        <span className="text-[13px] text-text">Enable outgoing email</span>
      </div>
      <div className="flex max-w-[520px] items-center gap-2.5">
        <Input
          value={testTo}
          onChange={(e) => setTestTo(e.target.value)}
          placeholder="you@example.com"
          className="flex-1"
        />
        <Button type="button" onClick={onSendTest} disabled={testSending || !testTo.trim()}>
          {testSending ? "Sending..." : "Send test"}
        </Button>
      </div>
      {testResult && (
        <p className={clsx("max-w-[520px] text-[13px]", testResult.ok ? "text-running" : "text-error")}>
          {testResult.ok ? "Test email sent." : (testResult.error ?? "the test email failed to send")}
        </p>
      )}
    </StepShell>
  );
}

function SsoStep({
  oidcEnvManaged,
  ssoEnabled,
  setSsoEnabled,
  issuer,
  setIssuer,
  clientId,
  setClientId,
  clientSecret,
  setClientSecret,
  clientSecretSet,
  scopes,
  setScopes,
  oidcAllowedDomains,
  setOidcAllowedDomains,
  callbackUrl,
  signupEnabled,
  setSignupEnabled,
  signupAllowedDomains,
  setSignupAllowedDomains,
  signupDefaultRoleId,
  setSignupDefaultRoleId,
  roles,
}: {
  oidcEnvManaged: boolean;
  ssoEnabled: boolean;
  setSsoEnabled: (v: boolean) => void;
  issuer: string;
  setIssuer: (v: string) => void;
  clientId: string;
  setClientId: (v: string) => void;
  clientSecret: string;
  setClientSecret: (v: string) => void;
  clientSecretSet: boolean;
  scopes: string;
  setScopes: (v: string) => void;
  oidcAllowedDomains: string;
  setOidcAllowedDomains: (v: string) => void;
  callbackUrl: string;
  signupEnabled: boolean;
  setSignupEnabled: (v: boolean) => void;
  signupAllowedDomains: string;
  setSignupAllowedDomains: (v: string) => void;
  signupDefaultRoleId: number | null;
  setSignupDefaultRoleId: (v: number | null) => void;
  roles: Role[];
}) {
  return (
    <StepShell
      title="How do people get in?"
      subcopy="Both are optional. Skip this and you create every account by hand, which is the right answer for a small team."
    >
      <div className="flex max-w-[560px] flex-col gap-3">
        <Card className="flex flex-col gap-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <div className="text-[14px] font-medium text-text-bright">Single sign-on</div>
              <div className="mt-0.5 text-[12.5px] text-text-dim">OpenID Connect against your identity provider.</div>
            </div>
            <Toggle checked={ssoEnabled} onChange={setSsoEnabled} label="Enable SSO" disabled={oidcEnvManaged} />
          </div>
          {oidcEnvManaged ? (
            <p className="text-[12.5px] text-text-dim">Configured through environment variables; managed there.</p>
          ) : (
            <div className="grid grid-cols-2 gap-3">
              <Field label="Issuer URL">
                <Input
                  value={issuer}
                  onChange={(e) => setIssuer(e.target.value)}
                  placeholder="https://accounts.example.com"
                />
              </Field>
              <Field label="Client ID">
                <Input value={clientId} onChange={(e) => setClientId(e.target.value)} />
              </Field>
              <Field label="Client secret">
                <Input
                  type="password"
                  value={clientSecret}
                  onChange={(e) => setClientSecret(e.target.value)}
                  placeholder={clientSecretSet ? "•••••••• (unchanged)" : "optional for public clients"}
                  autoComplete="new-password"
                />
              </Field>
              <Field label="Allowed domains">
                <Input
                  value={oidcAllowedDomains}
                  onChange={(e) => setOidcAllowedDomains(e.target.value)}
                  placeholder="(any) example.com"
                />
              </Field>
              <Field label="Scopes">
                <Input value={scopes} onChange={(e) => setScopes(e.target.value)} />
              </Field>
            </div>
          )}
          <div className="rounded-md border border-border-soft bg-code-bg px-2.5 py-2 font-mono text-[11.5px] text-text">
            Redirect URI &middot; {callbackUrl}
          </div>
        </Card>
        <Card className="flex flex-col gap-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <div className="text-[14px] font-medium text-text-bright">Public sign-up</div>
              <div className="mt-0.5 text-[12.5px] text-text-dim">
                Anyone on an allowed email domain can create their own account.
              </div>
            </div>
            <Toggle checked={signupEnabled} onChange={setSignupEnabled} label="Enable public sign-up" />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <Field label="Allowed domains">
              <Input
                value={signupAllowedDomains}
                onChange={(e) => setSignupAllowedDomains(e.target.value)}
                placeholder="(any) example.com"
              />
            </Field>
            <Field label="Default role">
              <Select
                value={signupDefaultRoleId ?? ""}
                onChange={(e) => setSignupDefaultRoleId(e.target.value ? Number(e.target.value) : null)}
              >
                <option value="">member (default)</option>
                {roles.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.name}
                  </option>
                ))}
              </Select>
            </Field>
          </div>
        </Card>
      </div>
    </StepShell>
  );
}

function InvitesStep({
  smtpReady,
  username,
  setUsername,
  email,
  setEmail,
  roleId,
  setRoleId,
  roles,
  onAdd,
  adding,
  invited,
  totalUsers,
}: {
  smtpReady: boolean;
  username: string;
  setUsername: (v: string) => void;
  email: string;
  setEmail: (v: string) => void;
  roleId: number | null;
  setRoleId: (v: number | null) => void;
  roles: Role[];
  onAdd: () => void;
  adding: boolean;
  invited: { username: string; note: string }[];
  totalUsers: number;
}) {
  return (
    <StepShell
      title="Invite your team"
      subcopy="Each person gets a workspace of their own the first time they open one."
    >
      <div className="flex max-w-[560px] flex-wrap items-end gap-2.5">
        <div className="min-w-[140px] flex-1">
          <Field label="Username">
            <Input value={username} onChange={(e) => setUsername(e.target.value)} placeholder="teammate" />
          </Field>
        </div>
        <div className="min-w-[180px] flex-1">
          <Field label="Email">
            <Input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="teammate@example.com"
            />
          </Field>
        </div>
        <Field label="Role">
          <Select value={roleId ?? ""} onChange={(e) => setRoleId(e.target.value ? Number(e.target.value) : null)}>
            {roles.map((r) => (
              <option key={r.id} value={r.id}>
                {r.name}
              </option>
            ))}
          </Select>
        </Field>
        <Button type="button" onClick={onAdd} disabled={adding}>
          {adding ? "Adding..." : "Add"}
        </Button>
      </div>
      {!smtpReady && (
        <p className="max-w-[520px] text-[12.5px] text-waiting">
          SMTP is not configured, so an invite does not send an email. A one-time password shows here instead; hand it
          to them yourself.
        </p>
      )}
      {invited.length > 0 && (
        <div className="flex max-w-[520px] flex-col gap-1.5">
          {invited.map((inv, i) => (
            <div
              key={i}
              className="flex items-center justify-between rounded-md border border-border-soft bg-surface-elevated px-3 py-2 text-[12.5px]"
            >
              <span className="text-text-bright">{inv.username}</span>
              <span className="font-mono text-text-dim">{inv.note}</span>
            </div>
          ))}
        </div>
      )}
      <Card className="flex max-w-[520px] flex-col gap-2.5">
        <SectionLabel>What they will see</SectionLabel>
        <p className="text-[13.5px] leading-relaxed text-text-dim">
          A three-step first run (what a workspace is, connect git, add an agent credential), then a workspace ready to
          launch a session in.
        </p>
      </Card>
      {totalUsers > 0 && (
        <p className="max-w-[520px] text-[12.5px] text-text-hint">
          {totalUsers} account{totalUsers === 1 ? "" : "s"} total so far.
        </p>
      )}
    </StepShell>
  );
}
