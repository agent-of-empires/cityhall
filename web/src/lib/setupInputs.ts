import { api } from "./api";
import type { SetupInputs } from "./setupProgress";

/** Reads every setting the checklist judges, in one round trip per area.
 *  An area that fails to load counts as unconfigured rather than failing the
 *  whole checklist, so a broken SMTP row cannot hide the rest of the setup. */
export async function fetchSetupInputs(mustChangePassword: boolean): Promise<SetupInputs> {
  const [workspaces, config, smtp, oidc, signup, users] = await Promise.all([
    api.getWorkspaceSettings().catch(() => null),
    api.getWorkspaceConfig().catch(() => null),
    api.getSmtpSettings().catch(() => null),
    api.getOidcSettings().catch(() => null),
    api.getSignupSettings().catch(() => null),
    api.listUsers().catch(() => null),
  ]);

  return {
    mustChangePassword,
    defaultVersion: workspaces?.default_version ?? null,
    agentCount: workspaces?.agents.length ?? 0,
    projectCount: config?.summary.projects.length ?? 0,
    emailReady: Boolean(smtp && (smtp.enabled || smtp.env_managed)),
    ssoReady: Boolean(oidc?.enabled || signup?.signup_enabled),
    userCount: users?.length ?? 0,
  };
}
