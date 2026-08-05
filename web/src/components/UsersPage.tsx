import { useCallback, useEffect, useState } from "react";
import { api, ApiError, can, type Me, type User } from "../lib/api";
import { PageBody, PageHeader } from "./AppShell";
import {
  Button,
  ErrorText,
  Input,
  StatusText,
  Tag,
  tableCardClass,
  tableHeadClass,
  tdClass,
  thClass,
  trClass,
} from "./ui";
import { UserDialog } from "./UserDialog";

export function UsersPage({ me }: { me: Me }) {
  const canWrite = can(me, "users.write");
  const [users, setUsers] = useState<User[]>([]);
  const [roleNames, setRoleNames] = useState<Record<number, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<{ user: User | null } | null>(null);
  const [filter, setFilter] = useState("");

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

  const q = filter.trim().toLowerCase();
  const filtered = q
    ? users.filter((u) => {
        const roleName = u.role_id !== null ? (roleNames[u.role_id] ?? "") : "";
        return (
          u.username.toLowerCase().includes(q) ||
          (u.email ?? "").toLowerCase().includes(q) ||
          roleName.toLowerCase().includes(q)
        );
      })
    : users;

  return (
    <>
      <PageHeader
        title="Users"
        meta={`${users.length} account${users.length === 1 ? "" : "s"}`}
        actions={
          canWrite && (
            <Button variant="primary" onClick={() => setDialog({ user: null })}>
              + New user
            </Button>
          )
        }
      />
      <PageBody className="space-y-3.5">
        {error && <ErrorText>{error}</ErrorText>}

        <Input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter by username, email or role"
          className="max-w-[320px]"
        />

        <div className={tableCardClass}>
          <table className="w-full text-sm">
            <thead>
              <tr className={tableHeadClass}>
                <th className={thClass}>Username</th>
                <th className={thClass}>Email</th>
                <th className={thClass}>Role</th>
                <th className={thClass}>Status</th>
                <th className={`${thClass} text-right`}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((u) => {
                const roleName = u.role_id !== null ? (roleNames[u.role_id] ?? `#${u.role_id}`) : null;
                return (
                  <tr key={u.id} className={trClass}>
                    <td className={`${tdClass} text-text-bright`}>{u.username}</td>
                    <td className={`${tdClass} font-mono text-[12.5px] text-text-dim`}>{u.email ?? "-"}</td>
                    <td className={tdClass}>
                      {roleName ? <Tag>{roleName}</Tag> : <span className="text-text-dim">-</span>}
                    </td>
                    <td className={tdClass}>
                      {u.must_change_password ? (
                        <StatusText tone="waiting" glyph="◐">
                          must change password
                        </StatusText>
                      ) : (
                        <StatusText tone="idle" glyph="○">
                          active
                        </StatusText>
                      )}
                    </td>
                    <td className={tdClass}>
                      <div className="flex justify-end gap-3">
                        {canWrite && (
                          <button
                            type="button"
                            onClick={() => setDialog({ user: u })}
                            className="font-mono text-xs text-text-faint hover:text-text"
                          >
                            edit
                          </button>
                        )}
                        {canWrite && u.id !== me.id && (
                          <button
                            type="button"
                            onClick={() => remove(u)}
                            className="font-mono text-xs text-error hover:text-error/80"
                          >
                            del
                          </button>
                        )}
                      </div>
                    </td>
                  </tr>
                );
              })}
              {filtered.length === 0 && (
                <tr>
                  <td colSpan={5} className={`${tdClass} py-8 text-center text-text-dim`}>
                    {users.length === 0 ? "No users yet." : "No users match this filter."}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </PageBody>

      {dialog && <UserDialog me={me} user={dialog.user} onClose={() => setDialog(null)} onSaved={load} />}
    </>
  );
}
