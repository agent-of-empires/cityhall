import { useCallback, useEffect, useState } from "react";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { api, ApiError, can, type Me, type User } from "../lib/api";
import { TopBar } from "./TopBar";
import { Button } from "./ui";
import { UserDialog } from "./UserDialog";

export function UsersPage({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
  const canWrite = can(me, "users.write");
  const [users, setUsers] = useState<User[]>([]);
  const [roleNames, setRoleNames] = useState<Record<number, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<{ user: User | null } | null>(null);

  const load = useCallback(async () => {
    try {
      setUsers(await api.listUsers());
      setError(null);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "could not load users");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!can(me, "roles.read")) return;
    api
      .listRoles()
      .then((rs) => setRoleNames(Object.fromEntries(rs.map((r) => [r.id, r.name]))))
      .catch(() => {});
  }, [me]);

  async function remove(user: User) {
    if (!confirm(`Delete user "${user.username}"?`)) return;
    try {
      await api.deleteUser(user.id);
      await load();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "could not delete user");
    }
  }

  return (
    <div className="flex h-full flex-col">
      <TopBar me={me} onLogout={onLogout} />
      <main className="mx-auto w-full max-w-3xl flex-1 space-y-4 overflow-auto p-6">
        <div className="flex items-center justify-between">
          <h2 className="font-mono text-xs uppercase tracking-wider text-text-muted">Users</h2>
          {canWrite && (
            <Button variant="primary" onClick={() => setDialog({ user: null })} className="flex items-center gap-1.5">
              <Plus size={14} />
              New user
            </Button>
          )}
        </div>

        {error && <p className="text-sm text-status-error">{error}</p>}

        <div className="overflow-hidden rounded-lg border border-surface-700">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-surface-700 bg-surface-850 text-left font-mono text-xs uppercase tracking-wider text-text-muted">
                <th className="px-4 py-2.5 font-medium">Username</th>
                <th className="px-4 py-2.5 font-medium">Email</th>
                <th className="px-4 py-2.5 font-medium">Role</th>
                <th className="px-4 py-2.5 font-medium">Status</th>
                <th className="px-4 py-2.5 text-right font-medium">Actions</th>
              </tr>
            </thead>
            <tbody>
              {users.map((u) => (
                <tr key={u.id} className="border-b border-surface-800 last:border-0">
                  <td className="px-4 py-2.5 text-text-primary">{u.username}</td>
                  <td className="px-4 py-2.5 text-text-secondary">{u.email ?? "-"}</td>
                  <td className="px-4 py-2.5 text-text-secondary">
                    {u.role_id !== null ? (roleNames[u.role_id] ?? `#${u.role_id}`) : "-"}
                  </td>
                  <td className="px-4 py-2.5">
                    {u.must_change_password ? (
                      <span className="text-status-waiting">◐ must change password</span>
                    ) : (
                      <span className="text-text-muted">○ active</span>
                    )}
                  </td>
                  <td className="px-4 py-2.5">
                    <div className="flex justify-end gap-1">
                      {canWrite && (
                        <Button variant="ghost" onClick={() => setDialog({ user: u })} aria-label="Edit">
                          <Pencil size={14} />
                        </Button>
                      )}
                      {canWrite && u.id !== me.id && (
                        <Button variant="danger" onClick={() => remove(u)} aria-label="Delete">
                          <Trash2 size={14} />
                        </Button>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
              {users.length === 0 && (
                <tr>
                  <td colSpan={5} className="px-4 py-8 text-center text-text-muted">
                    No users yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </main>

      {dialog && <UserDialog me={me} user={dialog.user} onClose={() => setDialog(null)} onSaved={load} />}
    </div>
  );
}
