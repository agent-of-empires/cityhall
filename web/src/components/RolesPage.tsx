import { useCallback, useEffect, useState } from "react";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { api, ApiError, can, type Me, type PermissionEntry, type Role } from "../lib/api";
import { TopBar } from "./TopBar";
import { Button, ErrorText, Field, Input } from "./ui";

export function RolesPage({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
  const canWrite = can(me, "roles.write");
  const [roles, setRoles] = useState<Role[]>([]);
  const [catalog, setCatalog] = useState<PermissionEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<{ role: Role | null } | null>(null);

  const load = useCallback(async () => {
    try {
      const [rs, perms] = await Promise.all([api.listRoles(), api.listPermissions()]);
      setRoles(rs);
      setCatalog(perms);
      setError(null);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "could not load roles");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function remove(role: Role) {
    if (!confirm(`Delete role "${role.name}"?`)) return;
    try {
      await api.deleteRole(role.id);
      await load();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "could not delete role");
    }
  }

  return (
    <div className="flex h-full flex-col">
      <TopBar me={me} onLogout={onLogout} />
      <main className="mx-auto w-full max-w-3xl flex-1 space-y-4 overflow-auto p-6">
        <div className="flex items-center justify-between">
          <h2 className="font-mono text-xs uppercase tracking-wider text-text-muted">Roles</h2>
          {canWrite && (
            <Button variant="primary" onClick={() => setDialog({ role: null })} className="flex items-center gap-1.5">
              <Plus size={14} />
              New role
            </Button>
          )}
        </div>

        {error && <ErrorText>{error}</ErrorText>}

        <div className="overflow-hidden rounded-lg border border-surface-700">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-surface-700 bg-surface-850 text-left font-mono text-xs uppercase tracking-wider text-text-muted">
                <th className="px-4 py-2.5 font-medium">Name</th>
                <th className="px-4 py-2.5 font-medium">Permissions</th>
                <th className="px-4 py-2.5 font-medium">Users</th>
                <th className="px-4 py-2.5 text-right font-medium">Actions</th>
              </tr>
            </thead>
            <tbody>
              {roles.map((r) => (
                <tr key={r.id} className="border-b border-surface-800 last:border-0">
                  <td className="px-4 py-2.5 text-text-primary">
                    {r.name}
                    {r.is_system && <span className="ml-2 text-xs text-text-muted">built-in</span>}
                  </td>
                  <td className="px-4 py-2.5 text-text-secondary">
                    {r.permissions.includes("*") ? "all" : r.permissions.join(", ") || "none"}
                  </td>
                  <td className="px-4 py-2.5 text-text-secondary">{r.user_count}</td>
                  <td className="px-4 py-2.5">
                    <div className="flex justify-end gap-1">
                      {canWrite && r.name !== "admin" && (
                        <Button variant="ghost" onClick={() => setDialog({ role: r })} aria-label="Edit">
                          <Pencil size={14} />
                        </Button>
                      )}
                      {canWrite && !r.is_system && (
                        <Button variant="danger" onClick={() => remove(r)} aria-label="Delete">
                          <Trash2 size={14} />
                        </Button>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
              {roles.length === 0 && (
                <tr>
                  <td colSpan={4} className="px-4 py-8 text-center text-text-muted">
                    No roles yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </main>

      {dialog && <RoleDialog role={dialog.role} catalog={catalog} onClose={() => setDialog(null)} onSaved={load} />}
    </div>
  );
}

function RoleDialog({
  role,
  catalog,
  onClose,
  onSaved,
}: {
  role: Role | null; // null = create mode
  catalog: PermissionEntry[];
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const editing = role !== null;
  const isSystem = role?.is_system ?? false;
  const [name, setName] = useState(role?.name ?? "");
  const [description, setDescription] = useState(role?.description ?? "");
  const [perms, setPerms] = useState<Set<string>>(new Set(role?.permissions ?? []));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function toggle(key: string) {
    setPerms((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const permissions = [...perms];
      if (editing) {
        await api.updateRole(role.id, {
          // A built-in role's name is fixed.
          ...(isSystem ? {} : { name }),
          description: description || null,
          permissions,
        });
      } else {
        await api.createRole({ name, description: description || null, permissions });
      }
      await onSaved();
      onClose();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not save role");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/50 p-4" onClick={onClose}>
      <div
        onClick={(e) => e.stopPropagation()}
        className="w-[var(--width-dialog)] space-y-5 rounded-lg border border-surface-700 bg-surface-850 p-6"
      >
        <form onSubmit={submit} className="space-y-5">
          <h2 className="font-mono text-base font-medium text-text-bright">{editing ? "Edit role" : "New role"}</h2>
          <Field label="Name">
            <Input value={name} onChange={(e) => setName(e.target.value)} disabled={isSystem} autoFocus />
          </Field>
          <Field label="Description">
            <Input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="optional" />
          </Field>
          <div className="space-y-1.5">
            <span className="font-mono text-xs uppercase tracking-wider text-text-muted">Permissions</span>
            <div className="space-y-2 rounded-md border border-surface-700 bg-surface-950 p-3">
              {catalog.map((p) => (
                <label key={p.key} className="flex items-center gap-2 text-sm text-text-primary">
                  <input
                    type="checkbox"
                    checked={perms.has(p.key)}
                    onChange={() => toggle(p.key)}
                    className="h-4 w-4 accent-brand-500"
                  />
                  <span className="font-mono text-xs text-text-secondary">{p.key}</span>
                  <span className="text-text-muted">{p.description}</span>
                </label>
              ))}
            </div>
          </div>
          {error && <ErrorText>{error}</ErrorText>}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={busy}>
              {busy ? "Saving..." : "Save"}
            </Button>
          </div>
        </form>
      </div>
    </div>
  );
}
