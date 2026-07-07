export interface User {
  id: number;
  username: string;
  email: string | null;
  must_change_password: boolean;
}

export interface Me {
  id: number;
  username: string;
  email: string | null;
  must_change_password: boolean;
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
  listUsers: () => request<User[]>("/users"),
  createUser: (username: string, email: string | null, password: string) =>
    request<User>("/users", {
      method: "POST",
      body: JSON.stringify({ username, email, password }),
    }),
  updateUser: (id: number, patch: { username?: string; email?: string; password?: string }) =>
    request<User>(`/users/${id}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  deleteUser: (id: number) => request<{ deleted: boolean }>(`/users/${id}`, { method: "DELETE" }),
};
