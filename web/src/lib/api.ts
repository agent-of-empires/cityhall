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
};
