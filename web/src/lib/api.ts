export interface User {
  id: number;
  username: string;
  email: string | null;
  must_change_password: boolean;
  role_id: number | null;
}

export interface Me {
  id: number;
  username: string;
  email: string | null;
  must_change_password: boolean;
  role_id: number | null;
  role: string | null;
  permissions: string[];
  onboarding_dismissed: boolean;
}

/** Whether `me` holds a permission (used to gate UI). */
export function can(me: Me | null, permission: string): boolean {
  return !!me && me.permissions.includes(permission);
}

export interface Role {
  id: number;
  name: string;
  description: string | null;
  permissions: string[];
  is_system: boolean;
  created_at: string;
  user_count: number;
}

export interface PermissionEntry {
  key: string;
  description: string;
}

export interface CreateUserInput {
  username: string;
  email: string | null;
  // Omit/empty to generate a password (unless sendSetupEmail is set).
  password?: string;
  sendSetupEmail?: boolean;
  roleId?: number;
}

export interface CreateUserResponse extends User {
  generated_password: string | null;
}

export interface SmtpSettings {
  env_managed: boolean;
  enabled: boolean;
  host: string;
  port: number;
  encryption: string;
  username: string | null;
  from_address: string;
  from_name: string | null;
  password_set: boolean;
  secret_key_available: boolean;
}

export interface SmtpUpdate {
  host: string;
  port: number;
  encryption: string;
  username: string | null;
  // Omit to keep the stored password; set a value to replace it.
  password?: string;
  from_address: string;
  from_name: string | null;
  enabled: boolean;
}

export interface Providers {
  oidc: boolean;
  signup: boolean;
}

export interface SignupSettings {
  signup_enabled: boolean;
  signup_allowed_domains: string;
  signup_default_role_id: number | null;
}

export interface OidcSettings {
  env_managed: boolean;
  enabled: boolean;
  issuer: string;
  client_id: string;
  scopes: string;
  allowed_domains: string;
  client_secret_set: boolean;
  secret_key_available: boolean;
  callback_path: string;
}

export interface OidcUpdate {
  enabled: boolean;
  issuer: string;
  client_id: string;
  // Omit to keep the stored secret; set a value to replace it.
  client_secret?: string;
  scopes: string;
  allowed_domains: string;
}

export interface WorkspaceItem {
  user_id: number;
  username: string;
  status: "not_created" | "stopped" | "running" | "unknown";
  pinned_version: string | null;
  effective_version: string | null;
  last_active_at: string | null;
  provisioning: { message: string; failed: boolean } | null;
}

/// What the system figures describe. CityHall often runs in a container beside
/// the workspaces it reports on, so the scope is shown rather than assumed.
export type MetricScope = "host" | "cityhall_container";

export interface DiskUsage {
  path: string;
  mount_point: string;
  used_bytes: number;
  total_bytes: number;
}

export interface SystemUsage {
  scope: MetricScope;
  cpu_percent: number;
  cpu_count: number;
  memory_used_bytes: number;
  memory_total_bytes: number;
  /// CityHall's own cgroup limit when it is narrower than the system's. Never a
  /// replacement for the totals above.
  cgroup_memory: { used_bytes: number; limit_bytes: number } | null;
  /// Only set when SYSTEM_METRICS_DISK_PATH is configured.
  disk: DiskUsage | null;
}

export interface WorkspaceUsage {
  user_id: number;
  /// Share of one CPU, so a container on two cores reads 200. Not clamped.
  cpu_percent: number;
  memory_bytes: number;
  memory_limit_bytes: number | null;
}

export interface DashboardWorkspace {
  user_id: number;
  username: string;
  status: "not_created" | "stopped" | "running" | "unknown";
  effective_version: string | null;
  running_version: string | null;
  last_active_at: string | null;
  provisioning: { message: string; failed: boolean } | null;
}

export interface DashboardSummary {
  total_users: number;
  running: number;
  stopped: number;
  not_created: number;
  unknown: number;
  provisioning: number;
  usage_available: number;
}

export interface Dashboard {
  /// Null until the sampler's first tick, which reads as "collecting".
  sampled_at: string | null;
  stale: boolean;
  errors: string[];
  system: SystemUsage | null;
  /// False when the backend has no metrics source at all.
  usage_supported: boolean;
  usage: WorkspaceUsage[];
  workspaces: DashboardWorkspace[];
  summary: DashboardSummary;
  versions: { version: string; count: number }[];
}

export interface DashboardLayoutItem {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface DashboardLayout {
  schema_version: number;
  items: DashboardLayoutItem[];
  hidden: string[];
}

export interface MyWorkspace {
  status: "not_created" | "stopped" | "running" | "unknown";
  pinned_version: string | null;
  effective_version: string | null;
  proxy_origin: string;
}

/// Who decides whether a workspace sends aoe telemetry. The three values this
/// CityHall knows and can select; a stored policy is a plain string because a
/// newer CityHall may have written one that is none of these.
export type TelemetryPolicy = "user_choice" | "force_on" | "force_off";

/// One coding agent a workspace can be set up to arrive with. The server owns
/// this catalog; the client lists whatever comes back rather than hardcoding it.
export interface AvailableAgent {
  name: string;
  label: string;
}

export interface WorkspaceSettings {
  image_template: string;
  default_version: string | null;
  idle_stop_minutes: number;
  /// The stored policy, verbatim. Usually a `TelemetryPolicy`, but an opaque
  /// value written by a newer CityHall is returned as-is so a save can echo it
  /// back rather than flattening it; `effective_telemetry_policy` is where such
  /// a value reads as `user_choice`.
  telemetry_policy: string;
  /// Set when WORKSPACE_TELEMETRY_POLICY pins the policy for the deployment, in
  /// which case it wins over the stored one.
  telemetry_policy_override: TelemetryPolicy | null;
  /// What workspaces actually run under: the override, or the stored policy.
  effective_telemetry_policy: TelemetryPolicy;
  /// Selected agents. Empty means users install their own.
  agents: string[];
  /// Everything selectable. Response only, so it is not part of a save.
  available_agents: AvailableAgent[];
}

/// What a save sends: the settings an admin owns, without the two fields the
/// server derives.
export interface WorkspaceSettingsUpdate {
  image_template: string;
  default_version: string | null;
  idle_stop_minutes: number;
  /// A `TelemetryPolicy`, or the stored value echoed back unchanged to keep an
  /// opaque one. The server rejects any other string.
  telemetry_policy: string;
  /// Recreate every running workspace so a saved policy applies now, which ends
  /// whatever their users are running. Stopped workspaces never need it.
  restart_running: boolean;
  /// The agents a workspace should arrive with. Absent would preserve whatever is
  /// stored, which is what an older client relies on; this one always sends the
  /// selection, so it is required here.
  agents: string[];
}

/// The aoe config bundle every workspace is provisioned with. Opaque TOML:
/// aoe owns the format, CityHall stores and serves it.
export interface WorkspaceConfig {
  bundle: string;
  updated_at: string | null;
  summary: {
    schema_version: number | null;
    settings_count: number;
    projects: string[];
  };
}

/// The admin setup checklist/wizard's persisted state: which steps have been
/// dismissed as handled or not needed, and whether the wizard itself has been
/// finished.
export interface SetupState {
  dismissed_steps: string[];
  wizard_finished: boolean;
}

/// A user's own git credential. The token is never returned, only whether one
/// is stored.
export interface GitCredential {
  host: string;
  username: string;
  token_set: boolean;
  secret_key_available: boolean;
}

/// A user's own git SSH key. The key is never returned, only whether one is
/// stored; known_hosts is, because a host's public key is public and re-pasting
/// it to change the key alone would be friction for nothing.
export interface GitSshKey {
  key_set: boolean;
  known_hosts: string;
  secret_key_available: boolean;
}

/// One agent-facing credential variable, whether or not a value is stored for
/// it. The server owns this catalog: the client renders whatever comes back
/// rather than hardcoding the variable list.
export interface AgentCredential {
  env_var: string;
  label: string;
  structured_view: boolean;
  // Non-null means this variable does not reach structured-view agents; the
  // string is the explanation to show the user.
  limitation: string | null;
  value_set: boolean;
  // false with value_set true means the stored value no longer decrypts.
  usable: boolean;
}

export interface AgentCredentials {
  secret_key_available: boolean;
  credentials: AgentCredential[];
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const res = await fetch(`/api${path}`, {
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    ...options,
  });
  if (!res.ok) {
    let message = res.statusText;
    try {
      const body = await res.json();
      if (body?.error) message = body.error;
    } catch {
      // Non-JSON error body; keep the status text.
    }
    throw new ApiError(res.status, message);
  }
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  return text ? (JSON.parse(text) as T) : (undefined as T);
}

export const api = {
  me: () => request<Me>("/auth/me"),
  login: (username: string, password: string) =>
    request<{ must_change_password: boolean }>("/auth/login", {
      method: "POST",
      body: JSON.stringify({ username, password }),
    }),
  logout: () => request<void>("/auth/logout", { method: "POST" }),
  changePassword: (current_password: string, new_password: string) =>
    request<Me>("/auth/change-password", {
      method: "POST",
      body: JSON.stringify({ current_password, new_password }),
    }),
  forgotPassword: (email: string) =>
    request<void>("/auth/forgot-password", {
      method: "POST",
      body: JSON.stringify({ email }),
    }),
  resetPassword: (token: string, new_password: string) =>
    request<void>("/auth/reset-password", {
      method: "POST",
      body: JSON.stringify({ token, new_password }),
    }),
  listUsers: () => request<User[]>("/users"),
  createUser: (input: CreateUserInput) =>
    request<CreateUserResponse>("/users", {
      method: "POST",
      body: JSON.stringify({
        username: input.username,
        email: input.email,
        password: input.password,
        send_setup_email: input.sendSetupEmail ?? false,
        role_id: input.roleId,
      }),
    }),
  updateUser: (id: number, patch: { username?: string; email?: string; password?: string; role_id?: number }) =>
    request<User>(`/users/${id}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  deleteUser: (id: number) => request<{ deleted: boolean }>(`/users/${id}`, { method: "DELETE" }),
  listRoles: () => request<Role[]>("/roles"),
  listPermissions: () => request<PermissionEntry[]>("/permissions"),
  createRole: (input: { name: string; description: string | null; permissions: string[] }) =>
    request<Role>("/roles", { method: "POST", body: JSON.stringify(input) }),
  updateRole: (id: number, patch: { name?: string; description?: string | null; permissions?: string[] }) =>
    request<Role>(`/roles/${id}`, { method: "PATCH", body: JSON.stringify(patch) }),
  deleteRole: (id: number) => request<{ deleted: boolean }>(`/roles/${id}`, { method: "DELETE" }),
  getSmtpSettings: () => request<SmtpSettings>("/settings/smtp"),
  updateSmtpSettings: (patch: SmtpUpdate) =>
    request<SmtpSettings>("/settings/smtp", {
      method: "PUT",
      body: JSON.stringify(patch),
    }),
  testSmtp: (to: string) =>
    request<{ ok: boolean; error: string | null }>("/settings/smtp/test", {
      method: "POST",
      body: JSON.stringify({ to }),
    }),
  providers: () => request<Providers>("/auth/providers"),
  register: (username: string, email: string, password: string) =>
    request<void>("/auth/register", {
      method: "POST",
      body: JSON.stringify({ username, email, password }),
    }),
  verifyEmail: (token: string) =>
    request<void>("/auth/verify-email", {
      method: "POST",
      body: JSON.stringify({ token }),
    }),
  getSignupSettings: () => request<SignupSettings>("/settings/signup"),
  updateSignupSettings: (patch: SignupSettings) =>
    request<SignupSettings>("/settings/signup", {
      method: "PUT",
      body: JSON.stringify(patch),
    }),
  getOidcSettings: () => request<OidcSettings>("/settings/oidc"),
  updateOidcSettings: (patch: OidcUpdate) =>
    request<OidcSettings>("/settings/oidc", {
      method: "PUT",
      body: JSON.stringify(patch),
    }),
  dashboard: () => request<Dashboard>("/dashboard"),
  getDashboardLayout: () => request<{ layout: DashboardLayout | null }>("/me/dashboard-layout"),
  saveDashboardLayout: (layout: DashboardLayout) =>
    request<void>("/me/dashboard-layout", { method: "PUT", body: JSON.stringify(layout) }),
  listWorkspaces: () => request<WorkspaceItem[]>("/workspaces"),
  myWorkspace: () => request<MyWorkspace>("/workspaces/me"),
  startWorkspace: (userId: number) => request<{ status: string }>(`/workspaces/${userId}/start`, { method: "POST" }),
  stopWorkspace: (userId: number) => request<{ status: string }>(`/workspaces/${userId}/stop`, { method: "POST" }),
  destroyWorkspace: (userId: number) => request<{ destroyed: boolean }>(`/workspaces/${userId}`, { method: "DELETE" }),
  setWorkspaceVersion: (userId: number, pinnedVersion: string | null, restart = false) =>
    request<{ pinned_version: string | null }>(`/workspaces/${userId}`, {
      method: "PATCH",
      body: JSON.stringify({ pinned_version: pinnedVersion, restart }),
    }),
  bulkSetWorkspaceVersion: (userIds: number[], pinnedVersion: string | null, restart = false) =>
    request<{ pinned_version: string | null }>("/workspaces", {
      method: "PATCH",
      body: JSON.stringify({ user_ids: userIds, pinned_version: pinnedVersion, restart }),
    }),
  listWorkspaceVersions: () =>
    request<{ latest: string | null; versions: string[]; stale: boolean }>("/workspaces/versions"),
  workspaceAccessUrl: (userId: number) =>
    request<{ url: string }>(`/workspaces/${userId}/access-url`, { method: "POST" }),
  getWorkspaceSettings: () => request<WorkspaceSettings>("/settings/workspaces"),
  updateWorkspaceSettings: (patch: WorkspaceSettingsUpdate) =>
    request<WorkspaceSettings>("/settings/workspaces", {
      method: "PUT",
      body: JSON.stringify(patch),
    }),
  getWorkspaceConfig: () => request<WorkspaceConfig>("/settings/workspace-config"),
  updateWorkspaceConfig: (bundle: string) =>
    request<WorkspaceConfig>("/settings/workspace-config", {
      method: "PUT",
      body: JSON.stringify({ bundle }),
    }),
  getSetupState: () => request<SetupState>("/settings/setup"),
  updateSetupState: (update: SetupState) =>
    request<SetupState>("/settings/setup", {
      method: "PUT",
      body: JSON.stringify(update),
    }),
  dismissOnboarding: () => request<{ onboarding_dismissed: boolean }>("/me/onboarding-dismissed", { method: "POST" }),
  getGitCredential: () => request<GitCredential>("/me/git-credential"),
  updateGitCredential: (patch: { host: string; username: string; token: string | null }) =>
    request<GitCredential>("/me/git-credential", {
      method: "PUT",
      body: JSON.stringify(patch),
    }),
  deleteGitCredential: () => request<GitCredential>("/me/git-credential", { method: "DELETE" }),
  getGitSshKey: () => request<GitSshKey>("/me/git-ssh-key"),
  updateGitSshKey: (patch: { key: string | null; known_hosts: string }) =>
    request<GitSshKey>("/me/git-ssh-key", {
      method: "PUT",
      body: JSON.stringify(patch),
    }),
  deleteGitSshKey: () => request<GitSshKey>("/me/git-ssh-key", { method: "DELETE" }),
  getAgentCredentials: () => request<AgentCredentials>("/me/agent-credentials"),
  getUserAgentCredentials: (userId: number) => request<AgentCredentials>(`/users/${userId}/agent-credentials`),
  updateAgentCredential: (envVar: string, value: string) =>
    request<AgentCredentials>(`/me/agent-credentials/${encodeURIComponent(envVar)}`, {
      method: "PUT",
      body: JSON.stringify({ value }),
    }),
  updateUserAgentCredential: (userId: number, envVar: string, value: string) =>
    request<AgentCredentials>(`/users/${userId}/agent-credentials/${encodeURIComponent(envVar)}`, {
      method: "PUT",
      body: JSON.stringify({ value }),
    }),
  deleteAgentCredential: (envVar: string) =>
    request<AgentCredentials>(`/me/agent-credentials/${encodeURIComponent(envVar)}`, { method: "DELETE" }),
  deleteUserAgentCredential: (userId: number, envVar: string) =>
    request<AgentCredentials>(`/users/${userId}/agent-credentials/${encodeURIComponent(envVar)}`, {
      method: "DELETE",
    }),
  restartMyWorkspace: () => request<void>("/workspaces/me/restart", { method: "POST" }),
};
