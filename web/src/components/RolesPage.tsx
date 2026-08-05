import { useCallback, useEffect, useState } from "react";
import { api, ApiError, can, type Me, type PermissionEntry, type Role } from "../lib/api";
import { PageBody, PageHeader } from "./AppShell";
import {
  Button,
  Checkbox,
  ErrorText,
  Field,
  Input,
  Modal,
  SectionLabel,
  Tag,
  tableCardClass,
  tableHeadClass,
  tdClass,
  thClass,
  trClass,
} from "./ui";

export function RolesPage({ me }: { me: Me }) {
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
    <>
      <PageHeader
        title="Roles"
        meta={`${roles.length} role${roles.length === 1 ? "" : "s"}`}
        actions={
          canWrite && (
            <Button variant="primary" onClick={() => setDialog({ role: null })}>
              + New role
            </Button>
          )
        }
      />
      <PageBody className="space-y-3.5">
        <p className="max-w-[600px] text-[13px] text-text-dim">
          Members hold <span className="font-mono text-text">workspaces.use</span> by default.{" "}
          <span className="font-mono text-text">workspaces.impersonate</span> is never implied, grant it only for
          support, and every use is written to the log.
        </p>

        {error && <ErrorText>{error}</ErrorText>}

        <div className={tableCardClass}>
          <table className="w-full text-sm">
            <thead>
              <tr className={tableHeadClass}>
                <th className={thClass}>Name</th>
                <th className={thClass}>Permissions</th>
                <th className={thClass}>Users</th>
                <th className={`${thClass} text-right`}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {roles.map((r) => (
                <tr key={r.id} className={trClass}>
                  <td className={tdClass}>
                    <div className="flex items-center gap-2">
                      <span className="text-text-bright">{r.name}</span>
                      {r.is_system && <span className="font-mono text-[10.5px] text-text-faint">built-in</span>}
                    </div>
                  </td>
                  <td className={tdClass}>
                    <div className="flex flex-wrap gap-1.5">
                      {r.permissions.includes("*") ? (
                        <Tag>all</Tag>
                      ) : r.permissions.length > 0 ? (
                        r.permissions.map((p) => <Tag key={p}>{p}</Tag>)
                      ) : (
                        <span className="text-text-dim">none</span>
                      )}
                    </div>
                  </td>
                  <td className={`${tdClass} font-mono text-text-dim`}>{r.user_count}</td>
                  <td className={tdClass}>
                    <div className="flex justify-end gap-3">
                      {canWrite && r.name !== "admin" && (
                        <button
                          type="button"
                          onClick={() => setDialog({ role: r })}
                          className="font-mono text-xs text-text-faint hover:text-text"
                        >
                          edit
                        </button>
                      )}
                      {canWrite && !r.is_system && (
                        <button
                          type="button"
                          onClick={() => remove(r)}
                          className="font-mono text-xs text-error hover:text-error/80"
                        >
                          del
                        </button>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
              {roles.length === 0 && (
                <tr>
                  <td colSpan={4} className={`${tdClass} py-8 text-center text-text-dim`}>
                    No roles yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </PageBody>

      {dialog && <RoleDialog role={dialog.role} catalog={catalog} onClose={() => setDialog(null)} onSaved={load} />}
    </>
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
    <Modal
      title={editing ? "Edit role" : "New role"}
      onClose={onClose}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" form="role-dialog-form" variant="primary" disabled={busy}>
            {busy ? "Saving..." : "Save"}
          </Button>
        </>
      }
    >
      <form id="role-dialog-form" onSubmit={submit} className="space-y-5">
        <Field label="Name">
          <Input value={name} onChange={(e) => setName(e.target.value)} disabled={isSystem} autoFocus />
        </Field>
        <Field label="Description">
          <Input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="optional" />
        </Field>
        <div className="space-y-1.5">
          <SectionLabel>Permissions</SectionLabel>
          <div className="space-y-2 rounded-md border border-border-soft bg-canvas p-3">
            {catalog.map((p) => (
              <Checkbox
                key={p.key}
                checked={perms.has(p.key)}
                onChange={() => toggle(p.key)}
                label={
                  <>
                    <span className="font-mono text-xs text-text-dim">{p.key}</span>{" "}
                    <span className="text-text-hint">{p.description}</span>
                  </>
                }
              />
            ))}
          </div>
        </div>
        {error && <ErrorText>{error}</ErrorText>}
      </form>
    </Modal>
  );
}
