import clsx from "clsx";
import { Boxes, ExternalLink, LogOut, Settings, Shield, Users } from "lucide-react";
import { useEffect, useState } from "react";
import { NavLink } from "react-router-dom";
import { api, can, type Me } from "../lib/api";
import { Button } from "./ui";

const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  clsx(
    "flex items-center gap-1.5 rounded-md px-2 py-1 text-sm transition-colors",
    isActive ? "text-text-primary" : "text-text-secondary hover:text-text-primary",
  );

export function TopBar({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
  // The user's workspace origin; only fetched when they may use one.
  const [workspaceOrigin, setWorkspaceOrigin] = useState<string | null>(null);

  useEffect(() => {
    if (!can(me, "workspaces.use")) return;
    api
      .myWorkspace()
      .then((w) => setWorkspaceOrigin(w.proxy_origin))
      .catch(() => {});
  }, [me]);

  async function logout() {
    await api.logout();
    await onLogout();
  }

  return (
    <header className="flex h-12 items-center justify-between border-b border-surface-700 px-4">
      <div className="flex items-center gap-4">
        <span className="font-mono text-sm font-medium tracking-wider text-text-bright">CityHall</span>
        <nav className="flex items-center gap-1">
          <NavLink to="/" end className={navLinkClass}>
            <Users size={14} />
            Users
          </NavLink>
          {can(me, "roles.read") && (
            <NavLink to="/roles" className={navLinkClass}>
              <Shield size={14} />
              Roles
            </NavLink>
          )}
          {can(me, "workspaces.read") && (
            <NavLink to="/workspaces" className={navLinkClass}>
              <Boxes size={14} />
              Workspaces
            </NavLink>
          )}
          {can(me, "settings.read") && (
            <NavLink to="/settings" className={navLinkClass}>
              <Settings size={14} />
              Settings
            </NavLink>
          )}
        </nav>
      </div>
      <div className="flex items-center gap-3">
        {workspaceOrigin && (
          <a
            // The exit param ends any admin access to another user's
            // workspace first, so this link always opens YOUR workspace.
            href={`${workspaceOrigin}/?cityhall_ws_exit=1`}
            target="_blank"
            rel="noreferrer"
            className="flex items-center gap-1.5 rounded-md px-2 py-1 text-sm text-text-secondary transition-colors hover:text-text-primary"
          >
            <ExternalLink size={14} />
            Open workspace
          </a>
        )}
        <span className="text-sm text-text-secondary">{me.username}</span>
        <Button variant="ghost" onClick={logout} className="flex items-center gap-1.5">
          <LogOut size={14} />
          Logout
        </Button>
      </div>
    </header>
  );
}
