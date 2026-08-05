/** The admin setup checklist.
 *
 * Only the password and the default version have a state that is unambiguously
 * "not set up yet". Everything else (agents, projects, email, SSO, extra users)
 * is optional, and an empty value is a legitimate choice, so a step also counts
 * as settled once an admin dismisses it. Dismissals live server side in
 * `setup_state`, which is why this module takes them as an input rather than
 * deriving them. */

export type SetupStepKey = "password" | "version" | "agents" | "projects" | "email" | "sso" | "invites";

export type SetupStep = {
  key: SetupStepKey;
  label: string;
  /** Where an admin goes to complete this step. */
  route: string;
  /** Whether the app refuses to run without it. */
  required: boolean;
};

export const SETUP_STEPS: SetupStep[] = [
  { key: "password", label: "Password", route: "/change-password", required: true },
  { key: "version", label: "aoe version", route: "/settings/workspace-defaults", required: true },
  { key: "agents", label: "Coding agents", route: "/settings/workspace-defaults", required: false },
  { key: "projects", label: "Projects", route: "/settings/workspace-config", required: false },
  { key: "email", label: "Email", route: "/settings/email", required: false },
  { key: "sso", label: "Sign-in & SSO", route: "/settings/access", required: false },
  { key: "invites", label: "Invite your team", route: "/users", required: false },
];

/** Everything the checklist reads, gathered from the settings endpoints. */
export type SetupInputs = {
  mustChangePassword: boolean;
  defaultVersion: string | null;
  agentCount: number;
  projectCount: number;
  /** SMTP enabled, or fixed by env vars. */
  emailReady: boolean;
  /** OIDC or self-signup enabled. */
  ssoReady: boolean;
  userCount: number;
};

export function stepConfigured(key: SetupStepKey, inputs: SetupInputs): boolean {
  switch (key) {
    case "password":
      return !inputs.mustChangePassword;
    case "version":
      return inputs.defaultVersion !== null;
    case "agents":
      return inputs.agentCount > 0;
    case "projects":
      return inputs.projectCount > 0;
    case "email":
      return inputs.emailReady;
    case "sso":
      return inputs.ssoReady;
    case "invites":
      return inputs.userCount > 1;
  }
}

export type SetupStepState = SetupStep & {
  /** The underlying setting has a value. */
  configured: boolean;
  /** Marked as handled, either configured or explicitly dismissed. */
  done: boolean;
};

export function setupSteps(inputs: SetupInputs, dismissed: string[]): SetupStepState[] {
  return SETUP_STEPS.map((step) => {
    const configured = stepConfigured(step.key, inputs);
    // A required step is only ever done by being configured. Dismissing one
    // would report a deployment as set up while no workspace can start.
    return { ...step, configured, done: configured || (!step.required && dismissed.includes(step.key)) };
  });
}

export function setupDoneCount(steps: SetupStepState[]): number {
  return steps.filter((step) => step.done).length;
}
